//! Ports of `lib/worker/WorkerProxy.java` and `lib/worker/SandboxedWorkerProxy.java`.

use std::collections::BTreeSet;
use std::fmt;
use std::io;
use std::sync::Arc;

use blaze_events::EventHandler;
use blaze_protos::blaze::worker::{WorkRequest, WorkResponse};
use blaze_vfs::{Path, PathFragment};
use indexmap::{IndexMap, IndexSet};
use parking_lot::Mutex;

use super::worker::{Worker, WorkerBase, WorkerError};
use super::worker_exec_root::WorkerExecRoot;
use super::worker_key::WorkerKey;
use super::worker_multiplexer::{WorkerMultiplexer, WorkerMultiplexerManager};
use super::worker_process_status::Status;
use crate::exec::tree_deleter::TreeDeleter;
use crate::sandbox::sandbox_helpers::{self, SandboxInputs, SandboxOutputs};

/// The parts of a `SandboxedWorkerProxy` its `WorkerProxy` superclass does not have.
struct SandboxedProxyParts {
    /// The sandbox directory for the current request, inside `workDir`.
    sandbox_dir: Path,
    sandbox_name: PathFragment,
    tree_deleter: Option<Arc<dyn TreeDeleter>>,
}

/// A proxy that talks to the multiplexer
// Java: com/google/devtools/build/lib/worker/WorkerProxy.java:31 WorkerProxy
pub struct WorkerProxy {
    base: WorkerBase,
    worker_multiplexer: Arc<WorkerMultiplexer>,
    /// The execution root of the worker. This is the CWD of the worker process.
    work_dir: Path,
    /// `Some` for a `SandboxedWorkerProxy`.
    sandboxed: Option<SandboxedProxyParts>,
    /// `synchronized destroy()`.
    destroy_lock: Mutex<()>,
}

impl WorkerProxy {
    // Java: com/google/devtools/build/lib/worker/WorkerProxy.java:38 WorkerProxy.<init>
    pub fn new(
        worker_key: Arc<WorkerKey>,
        worker_id: i32,
        log_file: Path,
        worker_multiplexer: Arc<WorkerMultiplexer>,
        work_dir: Path,
    ) -> WorkerProxy {
        // Worker proxies of the same multiplexer share a WorkerProcessStatus.
        let status = worker_multiplexer.get_status().clone();
        WorkerProxy {
            base: WorkerBase::new(worker_key, worker_id, log_file, status),
            worker_multiplexer,
            work_dir,
            sandboxed: None,
            destroy_lock: Mutex::new(()),
        }
    }

    /// A multiplex worker proxy with sandboxing. The multiplexer process runs in `workDir`, while
    /// each proxy has a fixed subdir where it sets up its files. The subdir is then passed to the
    /// worker in `WorkRequest#sandbox_dir`. The worker implementation is responsible for reading
    /// from and writing to that subdir only.
    // Java: com/google/devtools/build/lib/worker/SandboxedWorkerProxy.java:47 SandboxedWorkerProxy.<init>
    pub fn new_sandboxed(
        worker_key: Arc<WorkerKey>,
        worker_id: i32,
        log_file: Path,
        worker_multiplexer: Arc<WorkerMultiplexer>,
        work_dir: Path,
        tree_deleter: Option<Arc<dyn TreeDeleter>>,
    ) -> WorkerProxy {
        let sandbox_name = PathFragment::create(&format!(
            "__sandbox/{}/{}",
            worker_id,
            worker_key.get_exec_root().get_base_name()
        ));
        let mut proxy = WorkerProxy::new(
            worker_key,
            worker_id,
            log_file,
            worker_multiplexer,
            work_dir,
        );
        proxy.sandboxed = Some(SandboxedProxyParts {
            sandbox_dir: proxy.work_dir.get_relative(&sandbox_name),
            sandbox_name,
            tree_deleter,
        });
        proxy
    }
}

impl Worker for WorkerProxy {
    fn base(&self) -> &WorkerBase {
        &self.base
    }

    // Java: com/google/devtools/build/lib/worker/WorkerProxy.java:49 WorkerProxy.getCgroup
    fn get_cgroup(&self) -> Option<()> {
        // WorkerProxy does not have a cgroup at the momemnt. Consider adding it to the
        // multiplexer and returning it here?
        None
    }

    // Java: com/google/devtools/build/lib/worker/WorkerProxy.java:56 WorkerProxy.isSandboxed
    // Java: com/google/devtools/build/lib/worker/SandboxedWorkerProxy.java:63 SandboxedWorkerProxy.isSandboxed
    fn is_sandboxed(&self) -> bool {
        self.sandboxed.is_some()
    }

    // Java: com/google/devtools/build/lib/worker/WorkerProxy.java:61 WorkerProxy.setReporter
    fn set_reporter(&self, reporter: Option<Arc<dyn EventHandler>>) {
        // We might have created this multiplexer after setting the reporter for existing
        // multiplexers
        self.worker_multiplexer.set_reporter(reporter);
    }

    // Java: com/google/devtools/build/lib/worker/WorkerProxy.java:67 WorkerProxy.prepareExecution
    // Java: com/google/devtools/build/lib/worker/SandboxedWorkerProxy.java:68 SandboxedWorkerProxy.prepareExecution
    fn prepare_execution(
        &self,
        input_files: &SandboxInputs,
        outputs: &SandboxOutputs,
        worker_files: &BTreeSet<PathFragment>,
        client_env: &IndexMap<String, String>,
    ) -> Result<(), WorkerError> {
        let Some(sandboxed) = &self.sandboxed else {
            self.worker_multiplexer
                .create_process(&self.work_dir, client_env)?;
            return Ok(());
        };
        self.worker_multiplexer.create_sandboxed_process(
            &self.work_dir,
            worker_files,
            input_files,
            &**sandboxed
                .tree_deleter
                .as_ref()
                .expect("NullPointerException: treeDeleter"),
            client_env,
        )?;

        sandboxed.sandbox_dir.create_directory_and_parents()?;
        let mut dirs_to_create: IndexSet<PathFragment> = IndexSet::new();
        let mut inputs_to_create: IndexSet<PathFragment> = IndexSet::new();

        sandbox_helpers::populate_inputs_and_dirs_to_create(
            std::iter::empty(),
            &mut inputs_to_create,
            &mut dirs_to_create,
            input_files
                .get_files()
                .keys()
                .chain(input_files.get_symlinks().keys()),
            outputs,
        );
        sandbox_helpers::clean_existing(
            &sandboxed
                .sandbox_dir
                .get_parent_directory()
                .expect("NullPointerException: sandboxDir has no parent"),
            input_files,
            &mut inputs_to_create,
            &mut dirs_to_create,
            &sandboxed.sandbox_dir,
            &**sandboxed
                .tree_deleter
                .as_ref()
                .expect("NullPointerException: treeDeleter"),
        )?;
        // Finally, create anything that is still missing. This is non-strict only for historical
        // reasons, we haven't seen what would break if we make it strict.
        sandbox_helpers::create_directories(
            &dirs_to_create,
            &sandboxed.sandbox_dir,
            /* strict= */ false,
        )?;
        WorkerExecRoot::create_inputs(&inputs_to_create, input_files, &sandboxed.sandbox_dir)?;
        Ok(())
    }

    /// Send the WorkRequest to multiplexer.
    // Java: com/google/devtools/build/lib/worker/WorkerProxy.java:86 WorkerProxy.putRequest
    // Java: com/google/devtools/build/lib/worker/SandboxedWorkerProxy.java:103 SandboxedWorkerProxy.putRequest
    fn put_request(&self, request: &WorkRequest) -> io::Result<()> {
        match &self.sandboxed {
            None => self.worker_multiplexer.put_request(request.clone()),
            Some(sandboxed) => {
                // Modifying the request on the way out is not great. The alternatives are having
                // the spawn runner ask the worker for the dir or making the spawn runner
                // understand the sandbox, dir structure, neither of which are nice either.
                let mut request = request.clone();
                request.sandbox_dir = sandboxed.sandbox_name.get_path_string().to_string();
                self.worker_multiplexer.put_request(request)
            }
        }
    }

    /// Wait for WorkResponse from multiplexer.
    // Java: com/google/devtools/build/lib/worker/WorkerProxy.java:92 WorkerProxy.getResponse
    fn get_response(&self, request_id: i32) -> Result<Option<WorkResponse>, WorkerError> {
        self.worker_multiplexer.get_response(request_id)
    }

    // Java: com/google/devtools/build/lib/worker/SandboxedWorkerProxy.java:113 SandboxedWorkerProxy.finishExecution
    fn finish_execution(
        &self,
        exec_root: &Path,
        outputs: &SandboxOutputs,
    ) -> Result<(), WorkerError> {
        self.base.status.maybe_update_status(Status::Alive);
        if let Some(sandboxed) = &self.sandboxed {
            sandbox_helpers::move_outputs(outputs, &sandboxed.sandbox_dir, exec_root)?;
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/worker/WorkerProxy.java:76 WorkerProxy.destroy
    // Java: com/google/devtools/build/lib/worker/SandboxedWorkerProxy.java:120 SandboxedWorkerProxy.destroy
    fn destroy(&self) {
        let _guard = self.destroy_lock.lock();
        if let Err(_e) = WorkerMultiplexerManager::remove_instance(&self.base.worker_key) {
            // Exception (logged).
        }
        if let Some(sandboxed) = &self.sandboxed {
            // Caught IOException while deleting workdir (logged).
            let _ = sandboxed.sandbox_dir.delete_tree();
        }
    }

    // Java: com/google/devtools/build/lib/worker/WorkerProxy.java:97 WorkerProxy.diedUnexpectedly
    fn died_unexpectedly(&self) -> bool {
        self.worker_multiplexer.died_unexpectedly()
    }

    // Java: com/google/devtools/build/lib/worker/WorkerProxy.java:102 WorkerProxy.getExitValue
    fn get_exit_value(&self) -> Option<i32> {
        self.worker_multiplexer.get_exit_value()
    }

    // Java: com/google/devtools/build/lib/worker/WorkerProxy.java:107 WorkerProxy.getRecordingStreamMessage
    fn get_recording_stream_message(&self) -> String {
        self.worker_multiplexer.get_recording_stream_message()
    }

    // Java: com/google/devtools/build/lib/worker/WorkerProxy.java:117 WorkerProxy.getProcessId
    fn get_process_id(&self) -> i64 {
        self.worker_multiplexer.get_process_id()
    }
}

impl fmt::Display for WorkerProxy {
    // Java: com/google/devtools/build/lib/worker/WorkerProxy.java:112 WorkerProxy.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} proxy worker #{}",
            self.base.worker_key.get_mnemonic(),
            self.base.worker_id
        )
    }
}
