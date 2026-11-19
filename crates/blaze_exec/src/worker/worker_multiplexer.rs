//! Ports of `lib/worker/WorkerMultiplexer.java` and `lib/worker/WorkerMultiplexerManager.java`.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::io;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, LazyLock, Weak};
use std::time::Duration;

use blaze_events::{Event, EventHandler};
use blaze_protos::blaze::worker::{WorkRequest, WorkResponse};
use blaze_protos::failure_details::{self, FailureDetail, failure_detail};
use blaze_shell::{JavaFile, Subprocess, SubprocessBuilder, SubprocessFactory};
use blaze_vfs::{Path, PathFragment};
use indexmap::{IndexMap, IndexSet};
use parking_lot::{Condvar, Mutex};

use super::error_message::JavaException;
use super::recording_input_stream::{PipeInputStream, RecordingInputStream};
use super::shutdown_hooks::{self, ShutdownHookId};
use super::worker_exec_root::WorkerExecRoot;
use super::worker_key::WorkerKey;
use super::worker_process_status::{Status, WorkerProcessStatus};
use super::worker_protocol::{
    JsonWorkerProtocol, ProtoWorkerProtocol, SharedRecordingStream, WorkerProtocolImpl,
};
use crate::actions::UserExecException;
use crate::actions::execution_requirements::WorkerProtocolFormat;
use crate::exec::tree_deleter::TreeDeleter;
use crate::sandbox::sandbox_helpers::{self, SandboxInputs, SandboxOutputs};

/// `java.util.concurrent.Semaphore` with interruptible `acquire()`.
pub(crate) struct Semaphore {
    permits: Mutex<i32>,
    cv: Condvar,
}

impl Semaphore {
    fn new(permits: i32) -> Semaphore {
        Semaphore {
            permits: Mutex::new(permits),
            cv: Condvar::new(),
        }
    }

    /// `acquire()`: `Err` when the thread is (or gets) interrupted.
    // PORT: the interrupt flag is polled every 10ms while waiting.
    fn acquire(&self) -> Result<(), ()> {
        if blaze_shell::thread_interrupt::interrupted() {
            return Err(());
        }
        let mut p = self.permits.lock();
        loop {
            if *p > 0 {
                *p -= 1;
                return Ok(());
            }
            self.cv.wait_for(&mut p, Duration::from_millis(10));
            if *p == 0 && blaze_shell::thread_interrupt::interrupted() {
                return Err(());
            }
        }
    }

    fn release(&self) {
        *self.permits.lock() += 1;
        self.cv.notify_all();
    }
}

/// `LinkedBlockingQueue<WorkRequest>` whose `take()` the sender thread's interrupt ends.
struct RequestQueue {
    queue: Mutex<(VecDeque<WorkRequest>, bool)>,
    cv: Condvar,
}

impl RequestQueue {
    fn add(&self, request: WorkRequest) {
        self.queue.lock().0.push_back(request);
        self.cv.notify_all();
    }

    /// `take()`: `None` when the taking thread was interrupted.
    fn take(&self) -> Option<WorkRequest> {
        let mut q = self.queue.lock();
        loop {
            if q.1 {
                q.1 = false;
                return None;
            }
            if let Some(r) = q.0.pop_front() {
                return Some(r);
            }
            self.cv.wait(&mut q);
        }
    }

    /// `requestSender.interrupt()`.
    fn interrupt(&self) {
        self.queue.lock().1 = true;
        self.cv.notify_all();
    }
}

/// The fields Java guards with the multiplexer's monitor.
struct MultiplexerState {
    /// The worker process that this WorkerMultiplexer should be talking to. This should only be
    /// set once, when creating a new process. If the process dies or its stdio streams get
    /// corrupted, the `WorkerMultiplexer` gets discarded as well and a new one gets created as
    /// needed.
    process: Option<Arc<dyn Subprocess>>,
    /// The implementation of the worker protocol (JSON or Proto).
    worker_protocol: Option<Arc<dyn WorkerProtocolImpl>>,
    /// InputStream from the worker process.
    recording_stream: Option<Arc<Mutex<RecordingInputStream>>>,
    /// For testing only, allow a way to fake subprocesses.
    subprocess_factory: Option<Arc<dyn SubprocessFactory>>,
    /// A separate thread that receives responses: its interrupt status.
    response_receiver: Option<blaze_shell::ThreadInterrupt>,
    /// The active Reporter object, non-null if `--worker_verbose` is set. This must be cleared at
    /// the end of a command execution.
    reporter: Option<Arc<dyn EventHandler>>,
    /// Shutdown hook to make sure we wait for the process to finish on JVM shutdown, to avoid
    /// creating zombie processes.
    shutdown_hook: Option<ShutdownHookId>,
    /// The workDir of the multiplexer. We should clean this up on destroy if it's a sandboxed
    /// multiplex worker.
    work_dir: Option<Path>,
}

/// An intermediate worker that sends requests and receives responses from the worker
/// processes. There is at most one of these per `WorkerKey`, corresponding to one worker
/// process. `WorkerMultiplexer` objects run in separate long-lived threads. `WorkerProxy` objects
/// call into them to send requests. When a worker process returns a `WorkResponse`,
/// `WorkerMultiplexer` wakes up the relevant `WorkerProxy` to retrieve the response.
// Java: com/google/devtools/build/lib/worker/WorkerMultiplexer.java:53 WorkerMultiplexer
pub struct WorkerMultiplexer {
    /// An ID for this multiplexer that can be used by sandboxed multiplex workers to generate
    /// their workdir.
    multiplexer_id: i32,
    /// A queue of `WorkRequest` instances that need to be sent to the worker. `WorkerProxy`
    /// instances add to this queue, while the requestSender subthread remove requests and send
    /// them to the worker. This prevents dynamic execution interrupts from corrupting the `stdin`
    /// of the worker process.
    pending_requests: RequestQueue,
    /// A map of `WorkResponse`s received from the worker process. They are stored in this map
    /// keyed by the request id until the corresponding `WorkerProxy` picks them up.
    worker_process_response: Mutex<HashMap<i32, WorkResponse>>,
    /// A map of semaphores corresponding to `WorkRequest`s. After sending the `WorkRequest`,
    /// `WorkerProxy` will wait on a semaphore to be released. `WorkerMultiplexer` is responsible
    /// for releasing the corresponding semaphore in order to signal `WorkerProxy` that the
    /// `WorkerResponse` has been received.
    response_checker: Mutex<HashMap<i32, Arc<Semaphore>>>,
    /// Status of the worker process.
    status: Arc<WorkerProcessStatus>,
    /// The log file of the actual running worker process. It is shared between all WorkerProxy
    /// instances for this multiplexer.
    log_file: Path,
    /// The worker key that this multiplexer is for.
    worker_key: Arc<WorkerKey>,
    state: Mutex<MultiplexerState>,
    this: Weak<WorkerMultiplexer>,
}

impl WorkerMultiplexer {
    // Java: com/google/devtools/build/lib/worker/WorkerMultiplexer.java:143 WorkerMultiplexer.<init>
    pub fn new(log_file: Path, worker_key: Arc<WorkerKey>, multiplexer_id: i32) -> Arc<WorkerMultiplexer> {
        Arc::new_cyclic(|this| WorkerMultiplexer {
            multiplexer_id,
            pending_requests: RequestQueue {
                queue: Mutex::new((VecDeque::new(), false)),
                cv: Condvar::new(),
            },
            worker_process_response: Mutex::new(HashMap::new()),
            response_checker: Mutex::new(HashMap::new()),
            status: Arc::new(WorkerProcessStatus::new()),
            log_file,
            worker_key,
            state: Mutex::new(MultiplexerState {
                process: None,
                worker_protocol: None,
                recording_stream: None,
                subprocess_factory: None,
                response_receiver: None,
                reporter: None,
                shutdown_hook: None,
                work_dir: None,
            }),
            this: this.clone(),
        })
    }

    /// Sets or clears the reporter for outputting verbose info.
    // Java: com/google/devtools/build/lib/worker/WorkerMultiplexer.java:151 WorkerMultiplexer.setReporter
    pub fn set_reporter(&self, reporter: Option<Arc<dyn EventHandler>>) {
        self.state.lock().reporter = reporter;
    }

    /// Reports a string to the user if reporting is enabled.
    // Java: com/google/devtools/build/lib/worker/WorkerMultiplexer.java:156 WorkerMultiplexer.report
    fn report(&self, s: &str) {
        let reporter = self.state.lock().reporter.clone();
        if let Some(reporter) = reporter {
            reporter.handle(&Event::info(s));
        }
    }

    // Java: com/google/devtools/build/lib/worker/WorkerMultiplexer.java:162 WorkerMultiplexer.getStatus
    pub fn get_status(&self) -> &Arc<WorkerProcessStatus> {
        &self.status
    }

    /// Creates a worker process corresponding to this `WorkerMultiplexer`, if it doesn't already
    /// exist. Also starts up the subthreads handling reading and writing requests and responses,
    /// and sets up the sandbox root dir with the required worker files.
    // Java: com/google/devtools/build/lib/worker/WorkerMultiplexer.java:171 WorkerMultiplexer.createSandboxedProcess
    pub fn create_sandboxed_process(
        &self,
        work_dir: &Path,
        worker_files: &BTreeSet<PathFragment>,
        input_files: &SandboxInputs,
        tree_deleter: &dyn TreeDeleter,
        client_env: &IndexMap<String, String>,
    ) -> io::Result<()> {
        // TODO: Make blaze clean remove the workdir.
        let mut st = self.state.lock();
        if st.process.is_none() {
            // This should be a once-only operation.
            work_dir.create_directory_and_parents()?;
            work_dir.delete_trees_below()?;
            let mut dirs_to_create: IndexSet<PathFragment> = IndexSet::new();
            let mut inputs_to_create: IndexSet<PathFragment> = IndexSet::new();
            sandbox_helpers::populate_inputs_and_dirs_to_create(
                std::iter::empty(),
                &mut inputs_to_create,
                &mut dirs_to_create,
                worker_files.iter(),
                &SandboxOutputs::get_empty_instance(),
            );
            sandbox_helpers::clean_existing(
                &work_dir
                    .get_parent_directory()
                    .expect("NullPointerException: workDir has no parent"),
                input_files,
                &mut inputs_to_create,
                &mut dirs_to_create,
                work_dir,
                tree_deleter,
            )?;
            sandbox_helpers::create_directories(&dirs_to_create, work_dir, /* strict= */ false)?;
            let allowed: HashSet<PathFragment> = worker_files.iter().cloned().collect();
            WorkerExecRoot::create_inputs(
                &inputs_to_create,
                &input_files.limited_copy(&allowed),
                work_dir,
            )?;
            self.create_process_locked(&mut st, work_dir, client_env)?;
        }
        Ok(())
    }

    /// Creates a worker process corresponding to this `WorkerMultiplexer`, if it doesn't already
    /// exist. Also starts up the subthreads handling reading and writing requests and responses.
    // Java: com/google/devtools/build/lib/worker/WorkerMultiplexer.java:203 WorkerMultiplexer.createProcess
    pub fn create_process(
        &self,
        work_dir: &Path,
        client_env: &IndexMap<String, String>,
    ) -> io::Result<()> {
        let mut st = self.state.lock();
        self.create_process_locked(&mut st, work_dir, client_env)
    }

    fn create_process_locked(
        &self,
        st: &mut MultiplexerState,
        work_dir: &Path,
        client_env: &IndexMap<String, String>,
    ) -> io::Result<()> {
        if st.process.is_none() {
            if self.status.is_killed() {
                return Err(
                    JavaException::io("Multiplexer destroyed before created process")
                        .into_io_error(),
                );
            }
            let weak = self.this.clone();
            st.shutdown_hook = Some(shutdown_hooks::add_shutdown_hook(Arc::new(move || {
                if let Some(m) = weak.upgrade() {
                    m.destroy_from_shutdown_hook();
                }
            })));
            let mut args: Vec<String> = self.worker_key.get_args().to_vec();
            let executable = JavaFile::new(&args[0]);
            if !executable.is_absolute() && executable.get_parent().is_some() {
                args[0] = JavaFile::new_child(Some(&JavaFile::new(work_dir.get_path_string())), &args[0])
                    .get_absolute_path();
            }
            let mut process_builder = match &st.subprocess_factory {
                Some(f) => SubprocessBuilder::with_factory(client_env, f.clone()),
                None => SubprocessBuilder::new(client_env),
            };
            process_builder.set_argv(args);
            process_builder.set_working_directory(Some(JavaFile::new(work_dir.get_path_string())));
            process_builder.set_stderr_file(JavaFile::new(self.log_file.get_path_string()));
            process_builder.set_env(Some(self.worker_key.get_env().clone()));
            let process: Arc<dyn Subprocess> = Arc::from(process_builder.start()?);
            st.process = Some(process.clone());
            self.status.maybe_update_status(Status::Alive);

            let stdout = process
                .get_input_stream()
                .unwrap_or_else(|| Box::new(io::empty()));
            let recording_stream = Arc::new(Mutex::new(RecordingInputStream::new(Box::new(
                PipeInputStream::new(stdout, &format!("multiplexer-stdout-{}", self.multiplexer_id)),
            ))));
            recording_stream.lock().start_recording(4096);
            st.recording_stream = Some(recording_stream.clone());
            if st.worker_protocol.is_none() {
                let stdin: Box<dyn io::Write + Send> = process
                    .get_output_stream()
                    .unwrap_or_else(|| Box::new(io::sink()));
                let stdout = Box::new(SharedRecordingStream(recording_stream));
                st.worker_protocol = Some(match self.worker_key.get_protocol_format() {
                    WorkerProtocolFormat::Json => Arc::new(JsonWorkerProtocol::new(stdin, stdout)),
                    WorkerProtocolFormat::Proto => {
                        Arc::new(ProtoWorkerProtocol::new(stdin, stdout))
                    }
                });
            }
            let id = format!(
                "{}-{}",
                self.worker_key.get_mnemonic(),
                self.worker_key.hash_code()
            );
            // Created multiplexer process %s for worker %s (logged).
            // TODO(larsrc): Consider moving sender/receiver threads into separate classes.
            let sender = self.this.upgrade().expect("multiplexer alive");
            let sender_process = process.clone();
            std::thread::Builder::new()
                .name(format!("multiplexer-request-sender-{id}"))
                .spawn(move || while sender_process.is_alive() && sender.send_request() {})?;
            let receiver = self.this.upgrade().expect("multiplexer alive");
            let receiver_process = process.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::Builder::new()
                .name(format!("multiplexer-response-receiver-{id}"))
                .spawn(move || {
                    let _ = tx.send(blaze_shell::ThreadInterrupt::current());
                    while receiver_process.is_alive() && receiver.read_response() {}
                })?;
            st.response_receiver = rx.recv().ok();
        } else if !st.process.as_ref().unwrap().is_alive() {
            return Err(JavaException::io("Process is dead").into_io_error());
        }
        Ok(())
    }

    /// Returns the path of the log file shared by all multiplex workers using this process.
    // Java: com/google/devtools/build/lib/worker/WorkerMultiplexer.java:276 WorkerMultiplexer.getLogFile
    pub fn get_log_file(&self) -> &Path {
        &self.log_file
    }

    /// Signals this object to destroy itself, including the worker process. The object might not
    /// be fully destroyed at the end of this call, but will terminate soon. This is considered a
    /// deliberate destruction.
    // Java: com/google/devtools/build/lib/worker/WorkerMultiplexer.java:285 WorkerMultiplexer.destroyMultiplexer
    pub fn destroy_multiplexer(&self) {
        let mut st = self.state.lock();
        if st.process.is_some() {
            self.destroy_process(&mut st);
        }
        if let Some(work_dir) = &st.work_dir {
            // Failed to delete workDir (logged).
            let _ = work_dir.delete_tree();
        } else if self.worker_key.is_sandboxed() {
            // No workDir was deleted for this sandboxed multiplex worker because the workDir was
            // never set or set to null (logged).
        }
        // The WorkerProcessStatus is only set as killed once all WorkerProxy instances are
        // destroyed.
        self.status.set_killed();
    }

    /// `destroyMultiplexer()` run by the shutdown hook. PORT: does not wait for the monitor a
    /// blocked caller may hold (see `SingleplexShared::destroy_from_shutdown_hook`).
    fn destroy_from_shutdown_hook(&self) {
        match self.state.try_lock_for(Duration::from_millis(100)) {
            Some(mut st) => {
                st.shutdown_hook = None;
                if st.process.is_some() {
                    self.destroy_process(&mut st);
                }
            }
            None => {
                if let Some(st) = self.state.try_lock() {
                    if let Some(p) = &st.process {
                        p.destroy_and_wait();
                    }
                }
            }
        }
    }

    /// Destroys the worker subprocess. This might block forever if the subprocess refuses to die.
    /// It is safe to call this multiple times.
    // Java: com/google/devtools/build/lib/worker/WorkerMultiplexer.java:309 WorkerMultiplexer.destroyProcess
    fn destroy_process(&self, st: &mut MultiplexerState) {
        let process = st.process.clone().expect("process");
        process.destroy();
        // waitFor(), retried on interrupt; the interrupt is preserved.
        process.wait_for_uninterruptibly();
        if let Some(hook) = st.shutdown_hook.take() {
            shutdown_hooks::remove_shutdown_hook(hook);
        }
        // Stop the subthreads only when the process is dead, or their loops will go on.
        self.pending_requests.interrupt();
        if let Some(receiver) = &st.response_receiver {
            receiver.interrupt();
        }
        // Might as well release any waiting workers
        for semaphore in self.response_checker.lock().values() {
            semaphore.release();
        }
    }

    /// Sends the WorkRequest to worker process. This method is called on the thread of a
    /// `WorkerProxy`, and so is subject to interrupts by dynamic execution.
    // Java: com/google/devtools/build/lib/worker/WorkerMultiplexer.java:345 WorkerMultiplexer.putRequest
    pub fn put_request(&self, request: WorkRequest) -> io::Result<()> {
        let st = self.state.lock();
        if !st.process.as_ref().expect("NullPointerException: process").is_alive() {
            return Err(JavaException::io(format!(
                "Attempting to send request {} to dead process",
                request.request_id
            ))
            .into_io_error());
        }
        if !request.cancel {
            self.response_checker
                .lock()
                .insert(request.request_id, Arc::new(Semaphore::new(0)));
        }
        self.pending_requests.add(request);
        Ok(())
    }

    /// Waits on a semaphore for the `WorkResponse` returned from worker process. This method is
    /// called on the thread of a `WorkerProxy`, and so is subject to interrupts by dynamic
    /// execution.
    // Java: com/google/devtools/build/lib/worker/WorkerMultiplexer.java:361 WorkerMultiplexer.getResponse
    pub fn get_response(&self, request_id: i32) -> Result<Option<WorkResponse>, super::worker::WorkerError> {
        let process = self.state.lock().process.clone().expect("NullPointerException: process");
        if !process.is_alive() {
            // If the process has died, all we can do is return what may already have been
            // returned.
            return Ok(self.worker_process_response.lock().get(&request_id).cloned());
        }

        let wait_for_response = self.response_checker.lock().get(&request_id).cloned();

        let Some(wait_for_response) = wait_for_response else {
            self.report(&format!("Null response semaphore for {request_id}"));
            // If there is no semaphore for this request, it probably failed to send, so we just
            // return what we got, probably nothing.
            return Ok(self.worker_process_response.lock().get(&request_id).cloned());
        };

        if wait_for_response.acquire().is_err() {
            return Err(super::worker::WorkerError::Interrupted);
        }

        self.response_checker.lock().remove(&request_id);
        let response = self.worker_process_response.lock().remove(&request_id);

        if response.is_none() && !process.is_alive() {
            return Err(super::worker::WorkerError::Io(
                JavaException::io(format!(
                    "Worker process for {} has died",
                    self.worker_key.get_mnemonic()
                ))
                .into_io_error(),
            ));
        }
        Ok(response)
    }

    /// Sends a single pending request, if there are any. Blocks until a request is available.
    ///
    /// This is only called by the `requestSender` thread and so cannot be interrupted by dynamic
    /// execution cancellation, but only by a call to `destroyProcess()`.
    // Java: com/google/devtools/build/lib/worker/WorkerMultiplexer.java:394 WorkerMultiplexer.sendRequest
    fn send_request(&self) -> bool {
        let Some(request) = self.pending_requests.take() else {
            return false;
        };
        let protocol = self.state.lock().worker_protocol.clone();
        let result = match protocol {
            Some(p) => p.put_request(&request),
            None => Err(JavaException::io("no protocol").into_io_error()),
        };
        if result.is_err() {
            // We can't know how much of the request was sent, so we have to assume the worker's
            // input now contains garbage, and this request is lost.
            // TODO(b/177637516): Signal that this action failed for presumably transient reasons.
            self.report(&format!("Failed to send request {}", request.request_id));
            let s = self.response_checker.lock().remove(&request.request_id);
            if let Some(s) = s {
                s.release();
            }
            // TODO(b/177637516): Leave process in a moribound state so pending responses can be
            // returned.
            let mut st = self.state.lock();
            self.destroy_process(&mut st);
            return false;
        }
        true
    }

    /// Reads a `WorkResponse` from worker process, puts that `WorkResponse` in
    /// `workerProcessResponse`, and releases the semaphore for the `WorkerProxy`.
    ///
    /// This is only called on the readResponses subthread and so cannot be interrupted by
    /// dynamic execution cancellation, but only by a call to `destroyProcess()`.
    ///
    /// Returns true if the worker is still in a consistent state.
    // Java: com/google/devtools/build/lib/worker/WorkerMultiplexer.java:429 WorkerMultiplexer.readResponse
    fn read_response(&self) -> bool {
        let protocol = self.state.lock().worker_protocol.clone();
        let parsed = match protocol {
            Some(p) => p.get_response(),
            None => Ok(None),
        };
        let parsed_response = match parsed {
            Ok(r) => r,
            Err(e) => {
                if e.kind() != io::ErrorKind::Interrupted {
                    self.report(&format!(
                        "Error while reading response from multiplexer process for {}: {}",
                        self.worker_key.get_mnemonic(),
                        JavaException::from_io_error(&e)
                    ));
                }
                // We can't know how much of the response was read, so we have to assume the
                // worker's output now contains garbage, and we can't reliably read any further
                // responses.
                let mut st = self.state.lock();
                self.destroy_process(&mut st);
                return false;
            }
        };

        // A null parsedResponse can only happen if the input stream is closed, in which case we
        // drop everything.
        let Some(parsed_response) = parsed_response else {
            self.report(&format!(
                "Multiplexer process for {} has closed its output stream",
                self.worker_key.get_mnemonic()
            ));
            let mut st = self.state.lock();
            self.destroy_process(&mut st);
            return false;
        };

        let request_id = parsed_response.request_id;
        self.worker_process_response
            .lock()
            .insert(request_id, parsed_response);

        // TODO(b/151767359): When allowing cancellation, just remove responses that have no
        // matching entry in responseChecker.
        let semaphore = self.response_checker.lock().get(&request_id).cloned();
        if let Some(semaphore) = semaphore {
            // This wakes up the WorkerProxy that should receive this response.
            semaphore.release();
        } else {
            self.report(&format!(
                "Multiplexer for {} found no semaphore",
                self.worker_key.get_mnemonic()
            ));
            self.worker_process_response.lock().remove(&request_id);
        }
        true
    }

    // Java: com/google/devtools/build/lib/worker/WorkerMultiplexer.java:479 WorkerMultiplexer.getRecordingStreamMessage
    pub fn get_recording_stream_message(&self) -> String {
        // Once we read junk, we can't trust the rest of the stream
        let recording = self
            .state
            .lock()
            .recording_stream
            .clone()
            .expect("NullPointerException: recordingStream");
        let mut r = recording.lock();
        r.read_remaining();
        r.get_recorded_data_as_string()
    }

    /// Returns true if this process has died for other reasons than a call to `#destroy()`.
    // Java: com/google/devtools/build/lib/worker/WorkerMultiplexer.java:488 WorkerMultiplexer.diedUnexpectedly
    pub fn died_unexpectedly(&self) -> bool {
        let st = self.state.lock();
        st.process.as_ref().is_some_and(|p| !p.is_alive()) && !self.status.is_killed()
    }

    /// Returns the exit value of multiplexer's process, if it has exited.
    // Java: com/google/devtools/build/lib/worker/WorkerMultiplexer.java:493 WorkerMultiplexer.getExitValue
    pub fn get_exit_value(&self) -> Option<i32> {
        let st = self.state.lock();
        match &st.process {
            Some(p) if !p.is_alive() => Some(p.exit_value()),
            _ => None,
        }
    }

    /// For testing only, to verify that maps are cleared after responses are reaped.
    // Java: com/google/devtools/build/lib/worker/WorkerMultiplexer.java:501 WorkerMultiplexer.noOutstandingRequests
    pub fn no_outstanding_requests(&self) -> bool {
        self.response_checker.lock().is_empty() && self.worker_process_response.lock().is_empty()
    }

    // Java: com/google/devtools/build/lib/worker/WorkerMultiplexer.java:506 WorkerMultiplexer.setProcessFactory
    pub fn set_process_factory(&self, factory: Arc<dyn SubprocessFactory>) {
        self.state.lock().subprocess_factory = Some(factory);
    }

    /// `pendingRequests.size()` (`@VisibleForTesting`).
    pub fn pending_request_count(&self) -> usize {
        self.pending_requests.queue.lock().0.len()
    }

    // Java: com/google/devtools/build/lib/worker/WorkerMultiplexer.java:511 WorkerMultiplexer.getProcessId
    pub fn get_process_id(&self) -> i64 {
        match &self.state.lock().process {
            None => -1,
            Some(p) => p.get_process_id(),
        }
    }

    // Java: com/google/devtools/build/lib/worker/WorkerMultiplexer.java:518 WorkerMultiplexer.getMultiplexerId
    pub fn get_multiplexer_id(&self) -> i32 {
        self.multiplexer_id
    }

    // Java: com/google/devtools/build/lib/worker/WorkerMultiplexer.java:522 WorkerMultiplexer.setWorkDir
    pub fn set_work_dir(&self, work_dir: Path) {
        self.state.lock().work_dir = Some(work_dir);
    }
}

/// Contains the WorkerMultiplexer instance and reference count.
// Java: com/google/devtools/build/lib/worker/WorkerMultiplexerManager.java:143 WorkerMultiplexerManager.InstanceInfo
struct InstanceInfo {
    worker_multiplexer: Arc<WorkerMultiplexer>,
    ref_count: i32,
}

/// A map from the hash of `WorkerKey` objects to the corresponding information about the
/// multiplexer instance.
// Java: com/google/devtools/build/lib/worker/WorkerMultiplexerManager.java:36 WorkerMultiplexerManager.multiplexerInstance
static MULTIPLEXER_INSTANCE: LazyLock<Mutex<HashMap<WorkerKey, InstanceInfo>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// A counter used to provide unique IDs across sandboxed multiplexer instances.
// Java: com/google/devtools/build/lib/worker/WorkerMultiplexerManager.java:48 WorkerMultiplexerManager.multiplexerIdCounter
static MULTIPLEXER_ID_COUNTER: AtomicI32 = AtomicI32::new(1);

/// A manager to instantiate and destroy multiplexers. There should only be one
/// `WorkerMultiplexer` corresponding to workers with the same `WorkerKey`. If the
/// `WorkerMultiplexer` has been constructed, other workers should point to the same one.
// Java: com/google/devtools/build/lib/worker/WorkerMultiplexerManager.java:32 WorkerMultiplexerManager
pub struct WorkerMultiplexerManager;

fn create_user_exec_exception(
    message: String,
    detailed_code: failure_details::worker::Code,
) -> UserExecException {
    UserExecException::new(FailureDetail {
        message,
        category: Some(failure_detail::Category::Worker(failure_details::Worker {
            code: detailed_code as i32,
        })),
    })
}

impl WorkerMultiplexerManager {
    /// Returns a `WorkerMultiplexer` instance to `WorkerProxy`. `WorkerProxy` objects with the same
    /// `WorkerKey` talk to the same `WorkerMultiplexer`. Also, record how many `WorkerProxy`
    /// objects are talking to this `WorkerMultiplexer`.
    // Java: com/google/devtools/build/lib/worker/WorkerMultiplexerManager.java:55 WorkerMultiplexerManager.getInstance
    pub fn get_instance(key: &Arc<WorkerKey>, log_file: &Path) -> Arc<WorkerMultiplexer> {
        let mut map = MULTIPLEXER_INSTANCE.lock();
        let instance_info = map.entry((**key).clone()).or_insert_with(|| InstanceInfo {
            worker_multiplexer: WorkerMultiplexer::new(
                log_file.clone(),
                key.clone(),
                MULTIPLEXER_ID_COUNTER.fetch_add(1, Ordering::SeqCst),
            ),
            ref_count: 0,
        });
        instance_info.ref_count += 1;
        instance_info.worker_multiplexer.clone()
    }

    // Java: com/google/devtools/build/lib/worker/WorkerMultiplexerManager.java:67 WorkerMultiplexerManager.beforeCommand
    pub fn before_command(reporter: Option<Arc<dyn EventHandler>>) {
        WorkerMultiplexerManager::set_reporter(reporter);
    }

    // Java: com/google/devtools/build/lib/worker/WorkerMultiplexerManager.java:71 WorkerMultiplexerManager.afterCommand
    pub fn after_command() {
        WorkerMultiplexerManager::set_reporter(None);
    }

    /// Sets the reporter for all existing multiplexer instances. This allows reporting problems
    /// encountered while fetching an instance, e.g. during WorkerProxy validation.
    // Java: com/google/devtools/build/lib/worker/WorkerMultiplexerManager.java:79 WorkerMultiplexerManager.setReporter
    fn set_reporter(reporter: Option<Arc<dyn EventHandler>>) {
        for m in MULTIPLEXER_INSTANCE.lock().values() {
            m.worker_multiplexer.set_reporter(reporter.clone());
        }
    }

    /// Removes a `WorkerProxy` instance and reference count since it is no longer in use.
    // Java: com/google/devtools/build/lib/worker/WorkerMultiplexerManager.java:86 WorkerMultiplexerManager.removeInstance
    pub fn remove_instance(key: &WorkerKey) -> Result<(), UserExecException> {
        let to_destroy = {
            let mut map = MULTIPLEXER_INSTANCE.lock();
            let Some(instance_info) = map.get_mut(key) else {
                return Err(create_user_exec_exception(
                    format!(
                        "Attempting to remove non-existent {} multiplexer instance.",
                        key.get_mnemonic()
                    ),
                    failure_details::worker::Code::MultiplexerInstanceRemovalFailure,
                ));
            };
            instance_info.ref_count -= 1;
            if instance_info.ref_count == 0 {
                let m = instance_info.worker_multiplexer.clone();
                map.remove(key);
                Some(m)
            } else {
                None
            }
        };
        // PORT: Java destroys the multiplexer while holding the manager's monitor; here it is
        // released first (destroying waits for the process, which must not block other keys).
        if let Some(m) = to_destroy {
            m.destroy_multiplexer();
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/worker/WorkerMultiplexerManager.java:103 WorkerMultiplexerManager.getMultiplexer
    pub fn get_multiplexer(key: &WorkerKey) -> Result<Arc<WorkerMultiplexer>, UserExecException> {
        match MULTIPLEXER_INSTANCE.lock().get(key) {
            None => Err(create_user_exec_exception(
                "Accessing non-existent multiplexer instance.".to_string(),
                failure_details::worker::Code::MultiplexerDoesNotExist,
            )),
            Some(i) => Ok(i.worker_multiplexer.clone()),
        }
    }

    // Java: com/google/devtools/build/lib/worker/WorkerMultiplexerManager.java:113 WorkerMultiplexerManager.getRefCount
    pub fn get_ref_count(key: &WorkerKey) -> Result<i32, UserExecException> {
        match MULTIPLEXER_INSTANCE.lock().get(key) {
            None => Err(create_user_exec_exception(
                "Accessing non-existent multiplexer instance.".to_string(),
                failure_details::worker::Code::MultiplexerDoesNotExist,
            )),
            Some(i) => Ok(i.ref_count),
        }
    }

    // Java: com/google/devtools/build/lib/worker/WorkerMultiplexerManager.java:123 WorkerMultiplexerManager.getInstanceCount
    pub fn get_instance_count() -> i32 {
        MULTIPLEXER_INSTANCE.lock().len() as i32
    }

    /// Resets the instances. For testing only.
    // Java: com/google/devtools/build/lib/worker/WorkerMultiplexerManager.java:166 WorkerMultiplexerManager.resetForTesting
    pub fn reset_for_testing() {
        let all: Vec<Arc<WorkerMultiplexer>> = MULTIPLEXER_INSTANCE
            .lock()
            .drain()
            .map(|(_, i)| i.worker_multiplexer)
            .collect();
        for m in all {
            m.destroy_multiplexer();
        }
    }

    /// Injects a given WorkerMultiplexer into the instance map with refcount 0. For testing only.
    // Java: com/google/devtools/build/lib/worker/WorkerMultiplexerManager.java:175 WorkerMultiplexerManager.injectForTesting
    pub fn inject_for_testing(key: &WorkerKey, multiplexer: Arc<WorkerMultiplexer>) {
        MULTIPLEXER_INSTANCE.lock().insert(
            key.clone(),
            InstanceInfo {
                worker_multiplexer: multiplexer,
                ref_count: 0,
            },
        );
    }
}
