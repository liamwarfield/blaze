//! Port of `lib/worker/Worker.java`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io;
use std::sync::Arc;

use blaze_events::EventHandler;
use blaze_protos::blaze::worker::{WorkRequest, WorkResponse};
use blaze_vfs::{Path, PathFragment};
use indexmap::IndexMap;

use super::worker_key::WorkerKey;
use super::worker_process_status::{Status, WorkerProcessStatus};
use crate::actions::UserExecException;
use crate::sandbox::sandbox_helpers::{SandboxInputs, SandboxOutputs};

/// What the worker methods throw: `IOException`, `InterruptedException` or (from
/// `prepareExecution`) `UserExecException`.
#[derive(Debug)]
pub enum WorkerError {
    Io(io::Error),
    Interrupted,
    UserExec(UserExecException),
}

/// An `io::Error` made by `sandbox_helpers::interrupted_error` is Java's `InterruptedException`
/// (Blaze's interrupt checks); every other one, `InterruptedIOException` included, is an
/// `IOException` (review exec-2, E2-7).
impl From<io::Error> for WorkerError {
    fn from(e: io::Error) -> Self {
        if crate::sandbox::sandbox_helpers::is_interrupted_error(&e) {
            return WorkerError::Interrupted;
        }
        WorkerError::Io(e)
    }
}

impl From<UserExecException> for WorkerError {
    fn from(e: UserExecException) -> Self {
        WorkerError::UserExec(e)
    }
}

/// The fields of the abstract `Worker`.
// Java: com/google/devtools/build/lib/worker/Worker.java:34 Worker
#[derive(Debug)]
pub struct WorkerBase {
    /// An unique identifier of the work process.
    pub worker_key: Arc<WorkerKey>,
    /// An unique ID of the worker. It will be used in WorkRequest and WorkResponse as well.
    pub worker_id: i32,
    /// The path of the log file for this worker.
    pub log_file: Path,
    pub status: Arc<WorkerProcessStatus>,
    /// `cgroup`: the worker's cgroup, set when its process is created.
    pub cgroup: parking_lot::Mutex<Option<Arc<dyn crate::sandbox::cgroups::Cgroup>>>,
}

impl WorkerBase {
    // Java: com/google/devtools/build/lib/worker/Worker.java:45 Worker.<init>
    pub fn new(
        worker_key: Arc<WorkerKey>,
        worker_id: i32,
        log_file: Path,
        status: Arc<WorkerProcessStatus>,
    ) -> WorkerBase {
        WorkerBase {
            worker_key,
            worker_id,
            log_file,
            status,
            cgroup: parking_lot::Mutex::new(None),
        }
    }
}

/// An abstract superclass for persistent workers. Workers execute actions in long-running
/// processes that can handle multiple actions.
///
/// Workers are compared by identity (Java's default `equals`): the worker id is unique per
/// process.
// Java: com/google/devtools/build/lib/worker/Worker.java:34 Worker
pub trait Worker: Send + Sync + fmt::Display {
    /// The common fields.
    fn base(&self) -> &WorkerBase;

    /// Returns a unique id for this worker. This is used to distinguish different worker
    /// processes in logs and messages.
    // Java: com/google/devtools/build/lib/worker/Worker.java:63 Worker.getWorkerId
    fn get_worker_id(&self) -> i32 {
        self.base().worker_id
    }

    /// Returns the path of the log file for this worker.
    // Java: com/google/devtools/build/lib/worker/Worker.java:68 Worker.getLogFile
    fn get_log_file(&self) -> &Path {
        &self.base().log_file
    }

    /// Returns the worker key of this worker
    // Java: com/google/devtools/build/lib/worker/Worker.java:73 Worker.getWorkerKey
    fn get_worker_key(&self) -> &Arc<WorkerKey> {
        &self.base().worker_key
    }

    // Java: com/google/devtools/build/lib/worker/Worker.java:77 Worker.getStatus
    fn get_status(&self) -> &Arc<WorkerProcessStatus> {
        &self.base().status
    }

    /// `getCgroup()`.
    // Java: com/google/devtools/build/lib/worker/Worker.java:81 Worker.getCgroup
    fn get_cgroup(&self) -> Option<Arc<dyn crate::sandbox::cgroups::Cgroup>> {
        self.base().cgroup.lock().clone()
    }

    // Java: com/google/devtools/build/lib/worker/Worker.java:85 Worker.getWorkerFilesCombinedHash
    fn get_worker_files_combined_hash(&self) -> &[u8] {
        self.base().worker_key.get_worker_files_combined_hash()
    }

    // Java: com/google/devtools/build/lib/worker/Worker.java:89 Worker.getWorkerFilesWithDigests
    fn get_worker_files_with_digests(&self) -> &BTreeMap<PathFragment, Vec<u8>> {
        self.base().worker_key.get_worker_files_with_digests()
    }

    /// Returns true if this worker is sandboxed.
    // Java: com/google/devtools/build/lib/worker/Worker.java:94 Worker.isSandboxed
    fn is_sandboxed(&self) -> bool;

    /// Sets the reporter this `Worker` should report anomalous events to, or clears it. We
    /// expect the reporter to be cleared at end of build.
    // Java: com/google/devtools/build/lib/worker/Worker.java:100 Worker.setReporter
    fn set_reporter(&self, _reporter: Option<Arc<dyn EventHandler>>) {}

    /// Performs the necessary steps to prepare for execution. Once this is done, the worker should
    /// be able to receive a WorkRequest without further setup.
    // Java: com/google/devtools/build/lib/worker/Worker.java:106 Worker.prepareExecution
    fn prepare_execution(
        &self,
        input_files: &SandboxInputs,
        outputs: &SandboxOutputs,
        worker_files: &BTreeSet<PathFragment>,
        client_env: &IndexMap<String, String>,
    ) -> Result<(), WorkerError>;

    /// Sends a WorkRequest to the worker.
    // Java: com/google/devtools/build/lib/worker/Worker.java:120 Worker.putRequest
    fn put_request(&self, request: &WorkRequest) -> io::Result<()>;

    /// Waits to receive a response from the worker. This method should return as soon as a
    /// response has been received, moving of files and cleanup should wait until
    /// finishExecution(). `Ok(None)` where Java returns null.
    // Java: com/google/devtools/build/lib/worker/Worker.java:131 Worker.getResponse
    fn get_response(&self, request_id: i32) -> Result<Option<WorkResponse>, WorkerError>;

    /// Does whatever cleanup may be required after execution is done.
    // Java: com/google/devtools/build/lib/worker/Worker.java:139 Worker.finishExecution
    fn finish_execution(
        &self,
        _exec_root: &Path,
        _outputs: &SandboxOutputs,
    ) -> Result<(), WorkerError> {
        self.get_status().maybe_update_status(Status::Alive);
        Ok(())
    }

    /// Destroys this worker. Once this has been called, we assume it's safe to clean up related
    /// directories.
    // Java: com/google/devtools/build/lib/worker/Worker.java:148 Worker.destroy
    fn destroy(&self);

    /// Returns true if this worker is dead but we didn't deliberately kill it.
    // Java: com/google/devtools/build/lib/worker/Worker.java:151 Worker.diedUnexpectedly
    fn died_unexpectedly(&self) -> bool;

    /// Returns the exit value of this worker's process, if it has exited.
    // Java: com/google/devtools/build/lib/worker/Worker.java:154 Worker.getExitValue
    fn get_exit_value(&self) -> Option<i32>;

    /// Returns the last message received on the InputStream, if an unparseable message has been
    /// received.
    // Java: com/google/devtools/build/lib/worker/Worker.java:160 Worker.getRecordingStreamMessage
    fn get_recording_stream_message(&self) -> String;

    /// Returns process id of the worker, if the process already started. Otherwise returns -1.
    // Java: com/google/devtools/build/lib/worker/Worker.java:163 Worker.getProcessId
    fn get_process_id(&self) -> i64;
}

/// `java.lang.Thread.sleep(millis)`: `Err` if the thread is (or gets) interrupted; the flag is
/// cleared.
///
/// PORT: the interrupt flag is polled every millisecond.
pub(crate) fn java_sleep(millis: u64) -> Result<(), ()> {
    if blaze_shell::thread_interrupt::interrupted() {
        return Err(());
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(millis);
    loop {
        let now = std::time::Instant::now();
        if now >= deadline {
            return Ok(());
        }
        std::thread::sleep((deadline - now).min(std::time::Duration::from_millis(1)));
        if blaze_shell::thread_interrupt::interrupted() {
            return Err(());
        }
    }
}
