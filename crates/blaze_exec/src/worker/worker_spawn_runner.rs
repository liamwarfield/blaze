//! Port of `lib/worker/WorkerSpawnRunner.java`.

use std::collections::BTreeSet;
use std::io;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant, SystemTime};

use blaze_events::{EventHandler, JavaClock};
use blaze_optdefs::misc::worker_options::WorkerOptions;
use blaze_protos::blaze::worker::{Input, WorkRequest, WorkResponse};
use blaze_protos::failure_details::{self, FailureDetail, failure_detail};
use blaze_vfs::{Path, PathFragment};
use regex::Regex;

use super::error_message::{ErrorMessage, JavaException};
use super::worker::{Worker, WorkerError};
use super::worker_key::WorkerKey;
use super::worker_parser::{WorkerParser, WorkerParserError};
use super::worker_pool::WorkerPool;
use super::worker_process_metrics::WorkerProcessMetricsCollector;
use crate::actions::resource_manager::{
    AcquireResourcesError, InvalidateCause, ResourceHandle, ResourceManager, ResourcePriority,
};
use crate::actions::{
    ExecException, ExecKind, InputMetadataProvider, ResourceSet, RunfilesTree, Spawn, SpawnMetrics,
    SpawnResult, Status, UserExecException, input_metadata_provider, spawns,
};
use crate::exec::abstract_spawn_strategy::exec_io_exception;
use crate::exec::bin_tools::BinTools;
use crate::exec::local::local_env_provider::LocalEnvProvider;
use crate::exec::runfiles_tree_updater::{RunfilesTreeUpdater, UpdateRunfilesError};
use crate::exec::spawn_runner::{ProgressStatus, SpawnExecutionContext, SpawnRunner};
use crate::sandbox::sandbox_helpers::{self, SandboxInputs, SandboxOutputs};

// Java: com/google/devtools/build/lib/worker/WorkerSpawnRunner.java:85 WorkerSpawnRunner.ERROR_MESSAGE_PREFIX
pub const ERROR_MESSAGE_PREFIX: &str = "Worker strategy cannot execute this %s action, ";
// Java: com/google/devtools/build/lib/worker/WorkerSpawnRunner.java:87 WorkerSpawnRunner.REASON_NO_TOOLS
pub const REASON_NO_TOOLS: &str = "because the action has no tools";

/// The verbosity level implied by `--worker_verbose`. This value allows for manually setting some
/// only-slightly-verbose levels.
// Java: com/google/devtools/build/lib/worker/WorkerSpawnRunner.java:93 WorkerSpawnRunner.VERBOSE_LEVEL
const VERBOSE_LEVEL: i32 = 10;

/// The next work request ID to use. This field is static so we don't reuse work request IDs
/// across Bazel invocations. Although that shouldn't happen under normal circumstances because we
/// wait until a request finishes before exiting, it can happen if dynamic execution is enabled and
/// one branch beats the other in the race.
// Java: com/google/devtools/build/lib/worker/WorkerSpawnRunner.java:101 WorkerSpawnRunner.requestIdCounter
static REQUEST_ID_COUNTER: AtomicI32 = AtomicI32::new(1);

type WorkerCode = failure_details::worker::Code;

/// A spawn runner that launches Spawns the first time they are used in a persistent mode and
/// then shards work over all the processes.
// Java: com/google/devtools/build/lib/worker/WorkerSpawnRunner.java:83 WorkerSpawnRunner
pub struct WorkerSpawnRunner {
    exec_root: Path,
    reporter: Arc<dyn EventHandler>,
    resource_manager: Arc<ResourceManager>,
    runfiles_tree_updater: Arc<RunfilesTreeUpdater>,
    worker_options: Arc<WorkerOptions>,
    worker_parser: WorkerParser,
    metrics_collector: &'static WorkerProcessMetricsCollector,
}

/// What `execInWorker` and `executeRequest` throw.
enum RunnerError {
    /// `ExecException` (including `UserExecException` and `InterruptedException`).
    Exec(ExecException),
    /// `IOException`.
    Io(io::Error),
}

impl From<ExecException> for RunnerError {
    fn from(e: ExecException) -> Self {
        RunnerError::Exec(e)
    }
}

impl From<UserExecException> for RunnerError {
    fn from(e: UserExecException) -> Self {
        RunnerError::Exec(e.into())
    }
}

/// An `io::Error` made by `sandbox_helpers::interrupted_error` is Java's `InterruptedException`
/// (Blaze's interrupt checks); every other one, `InterruptedIOException` included, is an
/// `IOException` (review exec-2, E2-7).
impl From<io::Error> for RunnerError {
    fn from(e: io::Error) -> Self {
        if sandbox_helpers::is_interrupted_error(&e) {
            return RunnerError::Exec(ExecException::Interrupted);
        }
        RunnerError::Io(e)
    }
}

/// An `InterruptedIOException` stays an I/O failure; the thread's interrupt flag is set again.
// Java: com/google/devtools/build/lib/worker/WorkerSpawnRunner.java:695 WorkerSpawnRunner.restoreInterrupt
fn restore_interrupt(e: &io::Error) {
    if JavaException::is_interrupted_io_exception(e) {
        blaze_shell::thread_interrupt::interrupt_current();
    }
}

impl WorkerSpawnRunner {
    // Java: com/google/devtools/build/lib/worker/WorkerSpawnRunner.java:110 WorkerSpawnRunner.<init>
    pub fn new(
        exec_root: Path,
        workers: Arc<dyn WorkerPool>,
        reporter: Arc<dyn EventHandler>,
        local_env_provider: Arc<dyn LocalEnvProvider>,
        bin_tools: Arc<BinTools>,
        resource_manager: Arc<ResourceManager>,
        runfiles_tree_updater: Arc<RunfilesTreeUpdater>,
        worker_options: Arc<WorkerOptions>,
        worker_process_metrics_collector: &'static WorkerProcessMetricsCollector,
    ) -> WorkerSpawnRunner {
        let worker_parser = WorkerParser::new(
            exec_root.clone(),
            worker_options.clone(),
            local_env_provider,
            Some(bin_tools),
        );
        resource_manager.set_worker_pool(workers);
        worker_process_metrics_collector.set_clock(Arc::new(JavaClock));
        WorkerSpawnRunner {
            exec_root,
            reporter,
            resource_manager,
            runfiles_tree_updater,
            worker_options,
            worker_parser,
            metrics_collector: worker_process_metrics_collector,
        }
    }

    /// The body of `exec` up to the result.
    // Java: com/google/devtools/build/lib/worker/WorkerSpawnRunner.java:157 WorkerSpawnRunner.exec
    fn exec_inner(
        &self,
        spawn: &dyn Spawn,
        context: &dyn SpawnExecutionContext,
    ) -> Result<SpawnResult, RunnerError> {
        context.report(ProgressStatus::Scheduling(
            WorkerKey::make_worker_type_name(
                spawns::supports_multiplex_workers(spawn) && self.worker_options.worker_multiplex,
                context.speculating(),
            )
            .to_string(),
        ));
        if spawn.get_tool_files().is_empty() {
            return Err(create_user_exec_exception(
                ERROR_MESSAGE_PREFIX.replace("%s", &spawn.get_mnemonic()) + REASON_NO_TOOLS,
                WorkerCode::NoTools,
            )
            .into());
        }
        let start_time = SystemTime::now();
        let start_instant = Instant::now();
        let mut spawn_metrics;
        let response;
        {
            // PORT: skipped, profiling ("%s worker %s", "updateRunfiles").
            let mut runfiles_trees: Vec<Arc<dyn RunfilesTree>> = Vec::new();
            for tool_file in spawn.get_tool_files().to_list().iter() {
                if tool_file
                    .as_artifact()
                    .is_some_and(|a| a.is_runfiles_tree())
                {
                    runfiles_trees.push(
                        context
                            .get_input_metadata_provider()
                            .get_runfiles_metadata(tool_file)
                            .unwrap_or_else(|| {
                                panic!("NullPointerException: no runfiles metadata for {tool_file}")
                            })
                            .get_runfiles_tree()
                            .clone(),
                    );
                }
            }
            self.runfiles_tree_updater
                .update_runfiles(&runfiles_trees)
                .map_err(|e| match e {
                    UpdateRunfilesError::Exec(e) => RunnerError::Exec(e),
                    UpdateRunfilesError::Io(e) => RunnerError::Exec(exec_io_exception(&e)),
                })?;

            let input_file_cache = context.get_input_metadata_provider();
            // PORT: skipped, profiling (WORKER_SETUP "Setting up inputs").
            let input_files = sandbox_helpers::process_input_files(
                &context.get_input_mapping(
                    &PathFragment::create(""),
                    /* will_access_repeatedly= */ true,
                ),
                &self.exec_root,
            )?;
            let outputs = sandbox_helpers::get_outputs(spawn);
            let worker_config = match self.worker_parser.compute(spawn, context) {
                Ok(c) => c,
                Err(WorkerParserError::Exec(e)) => return Err(RunnerError::Exec(e)),
                Err(WorkerParserError::Io(e)) => return Err(RunnerError::from(e)),
            };
            let (key, flag_files) = worker_config.into_parts();
            spawn_metrics = SpawnMetrics::for_exec(ExecKind::Worker);
            spawn_metrics.input_files =
                (input_files.get_files().len() + input_files.get_symlinks().len()) as i64;
            response = self.exec_in_worker(
                spawn,
                Arc::new(key),
                context,
                &input_files,
                &outputs,
                &flag_files,
                input_file_cache,
                &mut spawn_metrics,
            )?;
            let out_err = context.get_file_out_err();
            // response.getOutputBytes().writeTo(outErr.getErrorStream())
            out_err.write_err(response.output.as_bytes())?;
        }
        let wall_time = start_instant.elapsed();
        let exit_code = response.exit_code;
        spawn_metrics.total_time_in_ms = to_ms(wall_time);
        let mut builder = self.get_spawn_result_builder(context);
        builder
            .set_exit_code(exit_code)
            .set_status(if exit_code == 0 {
                Status::Success
            } else {
                Status::NonZeroExit
            })
            .set_start_time(start_time)
            .set_wall_time_in_ms(to_ms(wall_time))
            .set_spawn_metrics(spawn_metrics);
        if exit_code != 0 {
            builder.set_failure_detail(FailureDetail {
                message: format!("worker spawn failed for {}", spawn.get_mnemonic()),
                category: Some(failure_detail::Category::Spawn(failure_details::Spawn {
                    code: failure_details::spawn::Code::NonZeroExit as i32,
                    spawn_exit_code: exit_code,
                    ..Default::default()
                })),
            });
        }
        Ok(builder.build())
    }

    // Java: com/google/devtools/build/lib/worker/WorkerSpawnRunner.java:243 WorkerSpawnRunner.createWorkRequest
    fn create_work_request(
        &self,
        spawn: &dyn Spawn,
        context: &dyn SpawnExecutionContext,
        input_files: &SandboxInputs,
        flagfiles: &[String],
        input_file_cache: &dyn InputMetadataProvider,
        key: &WorkerKey,
    ) -> Result<WorkRequest, RunnerError> {
        let mut request_builder = WorkRequest::default();
        for flagfile in flagfiles {
            expand_argument(input_files, flagfile, &mut request_builder)?;
        }

        let inputs = input_metadata_provider::expand_artifacts(
            context.get_input_metadata_provider(),
            spawn.get_input_files(),
            /* keep_empty_tree_artifacts= */ false,
            /* keep_runfiles_trees= */ false,
        );

        let virtual_input_digests = input_files.get_virtual_input_digests();
        for input in &inputs {
            let digest_bytes: Option<Vec<u8>> = if input.as_virtual_action_input().is_some() {
                Some(
                    virtual_input_digests
                        .get(input)
                        .expect("NullPointerException: missing metadata for virtual input")
                        .clone(),
                )
            } else {
                let metadata = input_file_cache
                    .get_input_metadata(input)?
                    .expect("NullPointerException: missing metadata for input");
                metadata.get_digest().map(<[u8]>::to_vec)
            };
            let digest: Vec<u8> = match digest_bytes {
                None => Vec::new(),
                Some(d) if d.is_empty() => Vec::new(),
                // ByteString.copyFromUtf8(HashCode.fromBytes(digestBytes).toString())
                Some(d) => hex::encode(d).into_bytes(),
            };
            request_builder.inputs.push(Input {
                path: blaze_util::string_encoding::internal_to_unicode(
                    &input.get_exec_path_string(),
                )
                .into_owned(),
                digest,
            });
        }
        if self.worker_options.worker_verbose {
            request_builder.verbosity = VERBOSE_LEVEL;
        }
        if key.is_multiplex() {
            request_builder.request_id = REQUEST_ID_COUNTER.fetch_add(1, Ordering::SeqCst);
        }
        Ok(request_builder)
    }

    // Java: com/google/devtools/build/lib/worker/WorkerSpawnRunner.java:375 WorkerSpawnRunner.execInWorker
    fn exec_in_worker(
        &self,
        spawn: &dyn Spawn,
        key: Arc<WorkerKey>,
        context: &dyn SpawnExecutionContext,
        input_files: &SandboxInputs,
        outputs: &SandboxOutputs,
        flag_files: &[String],
        input_file_cache: &dyn InputMetadataProvider,
        spawn_metrics: &mut SpawnMetrics,
    ) -> Result<WorkResponse, RunnerError> {
        let owner = spawn.get_resource_owner();
        let setup_inputs_stopwatch = Instant::now();
        let mut has_output_file_lock = false;
        // PORT: skipped, profiling (WORKER_SETUP "Preparing inputs").
        context.prefetch_inputs_and_wait()?;
        spawn_metrics.setup_time_in_ms = to_ms(setup_inputs_stopwatch.elapsed());

        let queue_stopwatch = Instant::now();
        let local_resources = spawn.get_local_resources()?;
        let resource_set = ResourceSet::create_with_worker_key(
            local_resources.get_resources().clone(),
            local_resources.get_local_test_count(),
            Some(key.clone()),
        );
        // Worker doesn't automatically return to pool after closing of the handle.
        let mut handle: Option<ResourceHandle> = None;
        // `workerOwner`: the worker this method still owns (`None` once finishWorkAsync took it).
        let mut worker_owner: Option<Arc<dyn Worker>> = None;
        let result: Result<WorkResponse, RunnerError> = (|| {
            let h = match self.resource_manager.acquire_resources_with_worker(
                owner,
                &resource_set,
                if context.speculating() {
                    ResourcePriority::DynamicWorker
                } else {
                    ResourcePriority::Local
                },
            ) {
                Ok(h) => h,
                Err(AcquireResourcesError::Exec(e)) => return Err(RunnerError::Exec(e)),
                Err(AcquireResourcesError::Io(e)) => return Err(RunnerError::from(e)),
            };
            let worker = h
                .get_worker()
                .cloned()
                .expect("NullPointerException: handle.getWorker()");
            handle = Some(h);
            worker_owner = Some(worker.clone());
            worker.set_reporter(if self.worker_options.worker_verbose {
                Some(self.reporter.clone())
            } else {
                None
            });
            let request = self.create_work_request(
                spawn,
                context,
                input_files,
                flag_files,
                input_file_cache,
                &key,
            )?;

            // We acquired a worker and resources -- mark that as queuing time.
            spawn_metrics.queue_time_in_ms = to_ms(queue_stopwatch.elapsed());
            let response = self.execute_request(
                spawn,
                context,
                input_files,
                outputs,
                &mut worker_owner,
                &key,
                &request,
                spawn_metrics,
                &mut handle,
            )?;

            let Some(response) = response else {
                return Err(create_empty_response_exception(worker.get_log_file()).into());
            };

            if response.was_cancelled {
                return Err(create_user_exec_exception(
                    format!(
                        "Received cancel response for {} without having cancelled",
                        response.request_id
                    ),
                    WorkerCode::FinishFailure,
                )
                .into());
            }

            // PORT: skipped, profiling (WORKER_COPYING_OUTPUTS "Worker #%d copying output files").
            let finish: Result<(), WorkerError> = match &worker_owner {
                Some(worker) => {
                    let process_outputs_stopwatch = Instant::now();
                    // Java's lockOutputFiles throws only InterruptedException; Blaze's
                    // ExecException (the dynamic strategy's) leaves as it is. Java passes a null
                    // FileOutErr here (review exec-2, E2-6).
                    // Java: com/google/devtools/build/lib/worker/WorkerSpawnRunner.java:443 WorkerSpawnRunner.execInWorker
                    context.lock_output_files(response.exit_code, &response.output, None)?;
                    has_output_file_lock = true;
                    (|| {
                        worker.finish_execution(&self.exec_root, outputs)?;
                        WorkerProcessMetricsCollector::instance()
                            .on_worker_finish_execution(worker.get_process_id());
                        spawn_metrics.process_outputs_time_in_ms =
                            to_ms(process_outputs_stopwatch.elapsed());
                        Ok(())
                    })()
                }
                None => Err(WorkerError::UserExec(create_user_exec_exception(
                    "The response finished successfully, but worker is taken by finishAsync"
                        .to_string(),
                    WorkerCode::FinishFailure,
                ))),
            };
            match finish {
                Ok(()) => {}
                Err(WorkerError::Io(e)) => {
                    restore_interrupt(&e);
                    let message = ErrorMessage::builder()
                        .message("IOException while finishing worker execution:")
                        .log_file(worker.get_log_file())
                        .exception(JavaException::from_io_error(&e))
                        .build()
                        .to_string();
                    return Err(
                        create_user_exec_exception(message, WorkerCode::FinishFailure).into(),
                    );
                }
                Err(WorkerError::Interrupted) => {
                    return Err(RunnerError::Exec(ExecException::Interrupted));
                }
                Err(WorkerError::UserExec(e)) => return Err(e.into()),
            }
            Ok(response)
        })();

        let result = match result {
            Ok(r) => Ok(r),
            Err(RunnerError::Io(e)) => {
                restore_interrupt(&e);
                let message = "IOException during worker execution:";
                Err(RunnerError::Exec(
                    create_user_exec_exception_with_io(&e, message, WorkerCode::BorrowFailure)
                        .into(),
                ))
            }
            Err(RunnerError::Exec(e @ (ExecException::User(_) | ExecException::Interrupted))) => {
                let worker = worker_owner.clone();
                let mut lock_error = None;
                if let (Some(h), Some(worker)) = (handle.take(), worker) {
                    let is_interrupted = matches!(e, ExecException::Interrupted);
                    let r = if is_interrupted && context.speculating() {
                        // When interrupted, we don't want to invalidate and kill the worker only
                        // to start it up later again (in dynamic execution). Just #close() the
                        // handle to release the acquired resources.
                        h.close()
                    } else {
                        h.invalidate_and_close(Some(match &e {
                            ExecException::Interrupted => InvalidateCause::Interrupted,
                            ExecException::User(u) => {
                                InvalidateCause::UserExec(match &u.failure_detail.category {
                                    Some(failure_detail::Category::Worker(w)) => {
                                        WorkerCode::try_from(w.code).ok()
                                    }
                                    _ => None,
                                })
                            }
                            _ => InvalidateCause::Other,
                        }))
                    };
                    if r.is_ok()
                        && !has_output_file_lock
                        && let Some(exit_value) = worker.get_exit_value()
                    {
                        // If the worker has died, we take the lock to a) fail earlier and b) have
                        // a chance to let the other dynamic execution branch take over if the
                        // error can be ignored.
                        // Java: com/google/devtools/build/lib/worker/WorkerSpawnRunner.java:485 WorkerSpawnRunner.execInWorker
                        // An InterruptedException of lockOutputFiles leaves the catch block in
                        // place of the original exception.
                        lock_error = context
                            .lock_output_files(exit_value, &e.get_message(), None)
                            .err();
                    }
                    // The original exception is more important / helpful, so we ignore errors of
                    // releasing the worker. `finally { workerOwner.setWorker(null) }`: the handle is
                    // already taken, so the outer finally does not close it again.
                    worker_owner.take();
                }
                Err(RunnerError::Exec(lock_error.unwrap_or(e)))
            }
            Err(other) => Err(other),
        };

        // if worker owner haven't initialized or we still haven't relased worker, than we need
        // to return resources.
        if let Some(h) = handle.take() {
            // PORT: Java's close() can throw IOException ("IOException while returning a worker
            // from the pool:", BORROW_FAILURE); Blaze's returns ExecException, propagated as is.
            h.close()?;
        }
        result
    }

    /// Executes worker request in worker, waits until the response is ready. Worker and
    /// resources should be allocated before call.
    // Java: com/google/devtools/build/lib/worker/WorkerSpawnRunner.java:505 WorkerSpawnRunner.executeRequest
    fn execute_request(
        &self,
        spawn: &dyn Spawn,
        context: &dyn SpawnExecutionContext,
        input_files: &SandboxInputs,
        outputs: &SandboxOutputs,
        worker_owner: &mut Option<Arc<dyn Worker>>,
        key: &WorkerKey,
        request: &WorkRequest,
        spawn_metrics: &mut SpawnMetrics,
        handle: &mut Option<ResourceHandle>,
    ) -> Result<Option<WorkResponse>, RunnerError> {
        context.report(ProgressStatus::Executing(
            key.get_worker_type_name().to_string(),
        ));
        let worker = worker_owner.clone().expect("worker");
        {
            // PORT: skipped, profiling (WORKER_SETUP "Worker #%d preparing execution").
            // We consider `prepareExecution` to be also part of setup.
            let prepare_execution_stopwatch = Instant::now();
            let worker_files: BTreeSet<PathFragment> = key
                .get_worker_files_with_digests()
                .keys()
                .cloned()
                .collect();
            match worker.prepare_execution(
                input_files,
                outputs,
                &worker_files,
                context.get_client_env(),
            ) {
                Ok(()) => {}
                Err(WorkerError::Io(e)) => {
                    restore_interrupt(&e);
                    let message = ErrorMessage::builder()
                        .message(
                            "IOException while preparing the execution environment of a worker:",
                        )
                        .log_file(worker.get_log_file())
                        .exception(JavaException::from_io_error(&e))
                        .build()
                        .to_string();
                    return Err(
                        create_user_exec_exception(message, WorkerCode::PrepareFailure).into(),
                    );
                }
                Err(WorkerError::Interrupted) => {
                    return Err(RunnerError::Exec(ExecException::Interrupted));
                }
                Err(WorkerError::UserExec(e)) => return Err(e.into()),
            }
            self.initialize_metrics(key, &*worker);
            spawn_metrics.setup_time_in_ms += to_ms(prepare_execution_stopwatch.elapsed());
        }

        let execution_stopwatch = Instant::now();
        // PORT: skipped, profiling (WORKER_SETUP "sending request").
        if let Err(e) = worker.put_request(request) {
            restore_interrupt(&e);
            let message = ErrorMessage::builder()
                .message(
                    "Worker process quit or closed its stdin stream when we tried to send a WorkRequest:",
                )
                .log_file(worker.get_log_file())
                .exception(JavaException::from_io_error(&e))
                .build()
                .to_string();
            return Err(create_user_exec_exception(message, WorkerCode::RequestFailure).into());
        }

        // PORT: skipped, profiling (WORKER_WORKING "Worker #%d working").
        let response = match worker.get_response(request.request_id) {
            Ok(r) => r,
            Err(WorkerError::Interrupted) => {
                if worker.is_sandboxed() {
                    // Sandboxed workers can safely finish their work async.
                    let h = handle.take().expect("handle");
                    self.finish_work_async(
                        worker.clone(),
                        request.clone(),
                        self.worker_options.worker_cancellation
                            && spawns::supports_worker_cancellation(spawn),
                        h,
                    );
                    *worker_owner = None;
                    self.resource_manager.release_resource_ownership();
                }
                return Err(RunnerError::Exec(ExecException::Interrupted));
            }
            Err(WorkerError::Io(e)) => {
                restore_interrupt(&e);
                // If protobuf or json reader couldn't parse the response, try to print whatever
                // the failing worker wrote to stdout - it's probably a stack trace or some kind of
                // error message that will help the user figure out why the compiler is failing.
                let recording_stream_message = worker.get_recording_stream_message();
                if recording_stream_message.is_empty() {
                    return Err(create_empty_response_exception(worker.get_log_file()).into());
                } else {
                    return Err(create_unparsable_response_exception(
                        &recording_stream_message,
                        worker.get_log_file(),
                        JavaException::from_io_error(&e),
                    )
                    .into());
                }
            }
            Err(WorkerError::UserExec(e)) => return Err(e.into()),
        };

        spawn_metrics.execution_wall_time_in_ms = to_ms(execution_stopwatch.elapsed());
        Ok(response)
    }

    // Java: com/google/devtools/build/lib/worker/WorkerSpawnRunner.java:592 WorkerSpawnRunner.initializeMetrics
    fn initialize_metrics(&self, worker_key: &WorkerKey, worker: &dyn Worker) {
        self.metrics_collector.register_worker(
            worker.get_worker_id(),
            worker.get_process_id(),
            worker.get_status(),
            worker_key.get_mnemonic(),
            worker_key.is_multiplex(),
            worker_key.is_sandboxed(),
            worker_key.hash_code(),
            worker.get_cgroup(),
        );
    }

    /// Starts a thread to collect the response from a worker when it's no longer of interest.
    ///
    /// This can happen either when we lost the race in dynamic execution or the build got
    /// interrupted. This takes ownership of the worker for purposes of returning it to the worker
    /// pool.
    // Java: com/google/devtools/build/lib/worker/WorkerSpawnRunner.java:611 WorkerSpawnRunner.finishWorkAsync
    fn finish_work_async(
        &self,
        worker: Arc<dyn Worker>,
        request: WorkRequest,
        can_cancel: bool,
        resource_handle: ResourceHandle,
    ) {
        let resource_manager = self.resource_manager.clone();
        let name = format!("AsyncFinish-Worker-{}", worker.get_worker_id());
        let _ = std::thread::Builder::new().name(name).spawn(move || {
            // A plain Java thread: an exception reaches the default uncaught exception handler.
            blaze_util::uncaught_exception::run_catching(move || {
                resource_manager.acquire_resource_ownership();
                let mut w = Some(worker);
                let result: Result<(), InvalidateCause> = (|| {
                    let worker = w.as_ref().unwrap();
                    if can_cancel {
                        let cancel_request = WorkRequest {
                            request_id: request.request_id,
                            cancel: true,
                            ..Default::default()
                        };
                        worker
                            .put_request(&cancel_request)
                            .map_err(|_| InvalidateCause::Io)?;
                    }
                    match worker.get_response(request.request_id) {
                        Ok(_) => Ok(()),
                        Err(WorkerError::Interrupted) => Err(InvalidateCause::Interrupted),
                        Err(_) => Err(InvalidateCause::Io),
                    }
                })();
                match result {
                    Ok(()) => {
                        // Error while returning worker to the pool. Could not do anything.
                        let _ = resource_handle.close();
                    }
                    Err(cause) => {
                        // If this happens, we either can't trust the output of the worker, or we got
                        // interrupted while handling being interrupted. In the latter case, let's stop
                        // trying and just destroy the worker. If it's a singleplex worker, there will
                        // be a dangling response that we don't want to keep trying to read, so we
                        // destroy the worker.
                        // The reaper thread can't do anything useful about errors here.
                        let _ = resource_handle.invalidate_and_close(Some(cause));
                        w = None;
                    }
                }
                let _ = w;
            });
        });
    }
}

impl SpawnRunner for WorkerSpawnRunner {
    // Java: com/google/devtools/build/lib/worker/WorkerSpawnRunner.java:157 WorkerSpawnRunner.exec
    fn exec(
        &self,
        spawn: &dyn Spawn,
        context: &dyn SpawnExecutionContext,
    ) -> Result<SpawnResult, ExecException> {
        match self.exec_inner(spawn, context) {
            Ok(r) => Ok(r),
            Err(RunnerError::Exec(e)) => Err(e),
            // An IOException leaves `exec` (AbstractSpawnStrategy's handler).
            Err(RunnerError::Io(e)) => Err(exec_io_exception(&e)),
        }
    }

    // Java: com/google/devtools/build/lib/worker/WorkerSpawnRunner.java:135 WorkerSpawnRunner.canExec
    fn can_exec(&self, spawn: &dyn Spawn) -> bool {
        if !spawns::supports_workers(spawn) && !spawns::supports_multiplex_workers(spawn) {
            return false;
        }
        // Note: `allowlist` is sorted, we could binary search.
        if let Some(allowlist) = &self.worker_options.allowlist
            && !allowlist.is_empty()
            && !allowlist.contains(&spawns::get_worker_key_mnemonic(spawn))
        {
            return false;
        }
        if spawn.get_tool_files().is_empty() {
            return false;
        }
        true
    }

    // Java: com/google/devtools/build/lib/worker/WorkerSpawnRunner.java:152 WorkerSpawnRunner.handlesCaching
    fn handles_caching(&self) -> bool {
        false
    }

    // Java: com/google/devtools/build/lib/worker/WorkerSpawnRunner.java:130 WorkerSpawnRunner.getName
    fn get_name(&self) -> String {
        "worker".to_string()
    }
}

fn to_ms(d: Duration) -> i32 {
    d.as_millis().min(i32::MAX as u128) as i32
}

/// `^@.*//.*` with `matches()`.
static EXTERNAL_REPOSITORY_LABEL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^@[^\n\r\u{85}\u{2028}\u{2029}]*//[^\n\r\u{85}\u{2028}\u{2029}]*$").unwrap()
});

// Java: com/google/devtools/build/lib/worker/WorkerSpawnRunner.java:334 WorkerSpawnRunner.isExternalRepositoryLabel
fn is_external_repository_label(arg: &str) -> bool {
    EXTERNAL_REPOSITORY_LABEL.is_match(arg)
}

/// `FileSystemUtils.readLines(path, UTF_8)`: lines split at `\n`, `\r\n` and `\r`, decoded as
/// UTF-8 (Unicode strings).
fn read_lines_utf8(path: &Path) -> io::Result<Vec<String>> {
    let bytes = crate::analysis::actions::file_system_utils::read_content(path)?;
    let text = String::from_utf8_lossy(&bytes);
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\n' => lines.push(std::mem::take(&mut current)),
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                lines.push(std::mem::take(&mut current));
            }
            _ => current.push(c),
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    Ok(lines)
}

/// Recursively expands arguments by replacing @filename args with the contents of the referenced
/// files. The @ itself can be escaped with @@. This deliberately does not expand --flagfile=
/// style arguments, because we want to get rid of the expansion entirely at some point in time.
///
/// Also check that the argument is not an external repository label, because they start with
/// `@` and are not flagfile locations.
// Java: com/google/devtools/build/lib/worker/WorkerSpawnRunner.java:307 WorkerSpawnRunner.expandArgument
pub fn expand_argument(
    inputs: &SandboxInputs,
    arg: &str,
    request_builder: &mut WorkRequest,
) -> io::Result<()> {
    if arg.starts_with('@') && !arg.starts_with("@@") && !is_external_repository_label(arg) {
        if blaze_shell::thread_interrupt::interrupted() {
            return Err(sandbox_helpers::interrupted_error(None));
        }
        let arg_value = &arg[1..];
        let path = inputs
            .get_files()
            .get(&PathFragment::create(arg_value))
            .cloned()
            .flatten();
        let Some(path) = path else {
            return Err(JavaException::io(format!(
                "Failed to read @-argument '{arg_value}': file is not a declared input"
            ))
            .into_io_error());
        };
        let result: io::Result<()> = (|| {
            for line in read_lines_utf8(&path)? {
                expand_argument(inputs, &line, request_builder)?;
            }
            Ok(())
        })();
        if let Err(e) = result {
            if sandbox_helpers::is_interrupted_error(&e) {
                return Err(e);
            }
            return Err(JavaException::with_cause(
                "java.io.IOException",
                format!(
                    "Failed to read @-argument '{}' from file '{}'.",
                    arg_value,
                    path.get_path_string()
                ),
                JavaException::from_io_error(&e),
            )
            .into_io_error());
        }
    } else {
        request_builder.arguments.push(arg.to_string());
    }
    Ok(())
}

// Java: com/google/devtools/build/lib/worker/WorkerSpawnRunner.java:338 WorkerSpawnRunner.createEmptyResponseException
fn create_empty_response_exception(logfile: &Path) -> UserExecException {
    let message = ErrorMessage::builder()
        .message("Worker process did not return a WorkResponse:")
        .log_file(logfile)
        .log_size_limit(8192)
        .build()
        .to_string();
    create_user_exec_exception(message, WorkerCode::NoResponse)
}

// Java: com/google/devtools/build/lib/worker/WorkerSpawnRunner.java:349 WorkerSpawnRunner.createUnparsableResponseException
fn create_unparsable_response_exception(
    recording_stream_message: &str,
    logfile: &Path,
    e: JavaException,
) -> UserExecException {
    let message = ErrorMessage::builder()
        .message(&format!(
            "Worker process returned an unparseable WorkResponse!\n\n\
             Did you try to print something to stdout? Workers aren't allowed to \
             do this, as it breaks the protocol between Bazel and the worker \
             process.\n\n\
             ---8<---8<--- Start of response ---8<---8<---\n\
             {recording_stream_message}\
             ---8<---8<--- End of response ---8<---8<---\n\n"
        ))
        .log_file(logfile)
        .log_size_limit(8192)
        .exception(e)
        .build()
        .to_string();
    create_user_exec_exception(message, WorkerCode::ParseResponseFailure)
}

// Java: com/google/devtools/build/lib/worker/WorkerSpawnRunner.java:697 WorkerSpawnRunner.createUserExecException
fn create_user_exec_exception_with_io(
    e: &io::Error,
    message: &str,
    detailed_code: WorkerCode,
) -> UserExecException {
    create_user_exec_exception(
        ErrorMessage::builder()
            .message(message)
            .exception(JavaException::from_io_error(e))
            .build()
            .to_string(),
        detailed_code,
    )
}

// Java: com/google/devtools/build/lib/worker/WorkerSpawnRunner.java:703 WorkerSpawnRunner.createUserExecException
fn create_user_exec_exception(message: String, detailed_code: WorkerCode) -> UserExecException {
    UserExecException::new(FailureDetail {
        message,
        category: Some(failure_detail::Category::Worker(failure_details::Worker {
            code: detailed_code as i32,
        })),
    })
}
