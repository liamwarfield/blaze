//! Port of `lib/exec/local/LocalSpawnRunner.java`.

use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use blaze_optdefs::misc::local_execution_options::LocalExecutionOptions;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail, spawn::Code};
use blaze_shell::{JavaFile, SubprocessBuilder, TerminationStatus};
use blaze_vfs::Path;

use crate::actions::resource_manager::{ResourceManager, ResourcePriority};
use crate::actions::{
    ExecException, ExecKind, POSIX_TIMEOUT_EXIT_CODE, RunfilesTree, Spawn, SpawnMetrics,
    SpawnResult, Status, spawns,
};
use crate::exec::abstract_spawn_strategy::exec_io_exception;
use crate::exec::bin_tools::BinTools;
use crate::exec::local::local_env_provider::LocalEnvProvider;
use crate::exec::spawn_runner::{ProgressStatus, SpawnExecutionContext, SpawnRunner};
use crate::runtime::ProcessWrapper;

// Java: com/google/devtools/build/lib/exec/local/LocalSpawnRunner.java:83 LocalSpawnRunner.LOCAL_EXEC_ERROR
const LOCAL_EXEC_ERROR: i32 = -1;

/// `LocalSpawnRunner.createActionTemp(execRoot)`, overridable as in Java (tests replace it).
pub type ActionTempCreator = Arc<dyn Fn(&Path) -> io::Result<Path> + Send + Sync>;

/// A class that runs local commands. Each request follows state transitions from "parsing" to
/// completion.
// Java: com/google/devtools/build/lib/exec/local/LocalSpawnRunner.java:80 LocalSpawnRunner
pub struct LocalSpawnRunner {
    exec_root: Path,
    resource_manager: Arc<ResourceManager>,
    host_name: String,
    local_execution_options: Arc<LocalExecutionOptions>,
    process_wrapper: Option<ProcessWrapper>,
    local_env_provider: Arc<dyn LocalEnvProvider>,
    bin_tools: Arc<BinTools>,
    // PORT: RunfilesTreeUpdater is not ported; a spawn with a runfiles tree input fails loudly in
    // update_runfiles.
    action_temp_creator: Option<ActionTempCreator>,
}

impl LocalSpawnRunner {
    /// `new LocalSpawnRunner(execRoot, localExecutionOptions, resourceManager, localEnvProvider,
    /// binTools, processWrapper, runfilesTreeUpdater)`.
    // Java: com/google/devtools/build/lib/exec/local/LocalSpawnRunner.java:101 LocalSpawnRunner.<init>
    pub fn new(
        exec_root: Path,
        local_execution_options: Arc<LocalExecutionOptions>,
        resource_manager: Arc<ResourceManager>,
        local_env_provider: Arc<dyn LocalEnvProvider>,
        bin_tools: Arc<BinTools>,
        process_wrapper: Option<ProcessWrapper>,
    ) -> LocalSpawnRunner {
        LocalSpawnRunner {
            exec_root,
            process_wrapper,
            local_execution_options,
            host_name: get_cached_short_host_name(),
            resource_manager,
            local_env_provider,
            bin_tools,
            action_temp_creator: None,
        }
    }

    /// Overrides `createActionTemp` (Java subclasses override the protected method).
    pub fn with_action_temp_creator(mut self, creator: ActionTempCreator) -> LocalSpawnRunner {
        self.action_temp_creator = Some(creator);
        self
    }

    // Java: com/google/devtools/build/lib/exec/local/LocalSpawnRunner.java:180 LocalSpawnRunner.createActionTemp
    fn create_action_temp(&self, exec_root: &Path) -> io::Result<Path> {
        if let Some(creator) = &self.action_temp_creator {
            return creator(exec_root);
        }
        exec_root.create_temp_directory("local-spawn-runner.")
    }

    /// `runfilesTreeUpdater.updateRunfiles(runfilesTrees)`.
    // Java: com/google/devtools/build/lib/exec/RunfilesTreeUpdater.java:71 RunfilesTreeUpdater.updateRunfiles
    fn update_runfiles(&self, runfiles_trees: &[Arc<dyn RunfilesTree>]) {
        if !runfiles_trees.is_empty() {
            blaze_util::unported!("RunfilesTreeUpdater.updateRunfiles");
        }
    }
}

impl SpawnRunner for LocalSpawnRunner {
    // Java: com/google/devtools/build/lib/exec/local/LocalSpawnRunner.java:120 LocalSpawnRunner.getName
    fn get_name(&self) -> String {
        "local".to_string()
    }

    // Java: com/google/devtools/build/lib/exec/local/LocalSpawnRunner.java:125 LocalSpawnRunner.exec
    fn exec(
        &self,
        spawn: &dyn Spawn,
        context: &dyn SpawnExecutionContext,
    ) -> Result<SpawnResult, ExecException> {
        let mut spawn_metrics = SpawnMetrics::for_exec(ExecKind::Local);
        let total_time_stopwatch = Instant::now();
        let setup_time_stopwatch = Instant::now();
        if spawns::should_prefetch_inputs_for_local_execution(spawn) {
            context.prefetch_inputs_and_wait()?;
        }
        spawn_metrics.setup_time_in_ms += to_ms(setup_time_stopwatch.elapsed());

        // PORT: profiling dropped.
        let owner = spawn.get_resource_owner();
        context.report(ProgressStatus::Scheduling(self.get_name()));

        let queue_stopwatch = Instant::now();
        let handle = self.resource_manager.acquire_resources(
            owner,
            &spawn.get_local_resources()?,
            if context.speculating() {
                ResourcePriority::DynamicStandalone
            } else {
                ResourcePriority::Local
            },
        )?;
        let body: Result<SpawnResult, ExecException> = (|| {
            spawn_metrics.queue_time_in_ms = to_ms(queue_stopwatch.elapsed());
            context.report(ProgressStatus::Executing(self.get_name()));
            if !self.local_execution_options.local_lockfree_output {
                // Without local-lockfree, we grab the lock before running the action, so we can't
                // check for failures while taking the lock.
                context.lock_output_files(0, "", context.get_file_out_err())?;
            }
            let result = SubprocessHandler::new(
                self,
                spawn,
                context,
                &mut spawn_metrics,
                total_time_stopwatch,
            )
            .run()?;
            if result.exit_code() != 0
                && self.local_execution_options.local_lockfree_output
                && context.speculating()
            {
                // We aren't going to write any output, but we should either abort the remote
                // branch early or let it finish if this error can be ignored. If the latter, this
                // call will throw DynamicInterruptedException.
                context.lock_output_files(result.exit_code(), "", context.get_file_out_err())?;
            }
            Ok(result)
        })();
        match body {
            Ok(result) => {
                handle.close()?;
                Ok(result)
            }
            Err(e) => {
                // try-with-resources: the handle is closed; its failure would be suppressed.
                let _ = handle.close();
                Err(e)
            }
        }
    }

    // Java: com/google/devtools/build/lib/exec/local/LocalSpawnRunner.java:171 LocalSpawnRunner.canExec
    fn can_exec(&self, spawn: &dyn Spawn) -> bool {
        !spawns::uses_path_mapping(spawn)
    }

    // Java: com/google/devtools/build/lib/exec/local/LocalSpawnRunner.java:176 LocalSpawnRunner.handlesCaching
    fn handles_caching(&self) -> bool {
        false
    }
}

/// `Ints.saturatedCast(duration.toMillis())` (`SpawnMetrics.Builder.toMs`).
// Java: com/google/devtools/build/lib/actions/SpawnMetrics.java:628 SpawnMetrics.Builder.toMs
fn to_ms(duration: Duration) -> i32 {
    duration.as_millis().min(i32::MAX as u128) as i32
}

// Java: com/google/devtools/build/lib/exec/local/LocalSpawnRunner.java:184 LocalSpawnRunner.SubprocessHandler
// PORT: the per-state timing (setState/stateTimes) and stepLog only feed logging; logging dropped.
struct SubprocessHandler<'a> {
    runner: &'a LocalSpawnRunner,
    spawn: &'a dyn Spawn,
    context: &'a dyn SpawnExecutionContext,
    spawn_metrics: &'a mut SpawnMetrics,
    total_time_stopwatch: Instant,
    /// If true, the local subprocess has already started, which means we need to clean up the
    /// output tree once we get interrupted.
    need_cleanup: bool,
}

impl<'a> SubprocessHandler<'a> {
    // Java: com/google/devtools/build/lib/exec/local/LocalSpawnRunner.java:203 LocalSpawnRunner.SubprocessHandler.<init>
    fn new(
        runner: &'a LocalSpawnRunner,
        spawn: &'a dyn Spawn,
        context: &'a dyn SpawnExecutionContext,
        spawn_metrics: &'a mut SpawnMetrics,
        total_time_stopwatch: Instant,
    ) -> SubprocessHandler<'a> {
        blaze_util::check_argument!(!spawn.get_arguments().is_empty());
        let _id = context.get_id();
        SubprocessHandler {
            runner,
            spawn,
            context,
            spawn_metrics,
            total_time_stopwatch,
            need_cleanup: false,
        }
    }

    // Java: com/google/devtools/build/lib/exec/local/LocalSpawnRunner.java:217 LocalSpawnRunner.SubprocessHandler.run
    fn run(&mut self) -> Result<SpawnResult, ExecException> {
        let local_retries_on_crash = self.runner.local_execution_options.local_retries_on_crash;
        if local_retries_on_crash == 0 {
            self.run_once()
        } else {
            let mut attempts = 0;
            loop {
                // Assume that any exceptions from runOnce() come from the Java side of things,
                // not the subprocess, so let them bubble up on first occurrence. In particular, we
                // need this to be true for InterruptedException to ensure that the dynamic
                // scheduler can stop us quickly.
                let retry_stopwatch = Instant::now();
                let result = self.run_once()?;
                if attempts == local_retries_on_crash
                    || !TerminationStatus::crashed(result.exit_code())
                {
                    return Ok(result);
                }
                // PORT: logging dropped ("Retrying crashed subprocess due to exit code ...").
                std::thread::sleep(Duration::from_millis(attempts as u64 * 1000));
                *self
                    .spawn_metrics
                    .retry_time_in_ms
                    .entry(result.exit_code())
                    .or_insert(0) += to_ms(retry_stopwatch.elapsed());
                attempts += 1;
            }
        }
    }

    // Java: com/google/devtools/build/lib/exec/local/LocalSpawnRunner.java:246 LocalSpawnRunner.SubprocessHandler.runOnce
    fn run_once(&mut self) -> Result<SpawnResult, ExecException> {
        match self.start() {
            Err(ExecException::Interrupted) => {
                self.maybe_cleanup_on_interrupt();
                // Logging the exception causes a lot of noise in builds using the dynamic
                // scheduler, and the information is not very interesting, so avoid that.
                Err(ExecException::Interrupted)
            }
            other => other,
        }
    }

    /// Parse the request and run it locally.
    // Java: com/google/devtools/build/lib/exec/local/LocalSpawnRunner.java:309 LocalSpawnRunner.SubprocessHandler.start
    fn start(&mut self) -> Result<SpawnResult, ExecException> {
        let runner = self.runner;
        let spawn = self.spawn;
        let context = self.context;
        let exec_root = &runner.exec_root;

        let mut spawn_result_builder = runner.get_spawn_result_builder(context);
        spawn_result_builder.set_executor_hostname(runner.host_name.clone());

        let out_err = context.get_file_out_err();
        let action_type = spawn.get_resource_owner().get_mnemonic();
        if let Some(allowed_local_action) = &runner.local_execution_options.allowed_local_action
            && !allowed_local_action.matches(&action_type)
        {
            // PORT: StringEncoding identity (unicodeToInternal).
            out_err
                .write_err(
                    format!(
                        "Action type {} is not allowed to run locally due to regex filter: {}\n",
                        action_type,
                        allowed_local_action.pattern()
                    )
                    .as_bytes(),
                )
                .map_err(|e| exec_io_exception(&e))?;
            self.spawn_metrics.total_time_in_ms = to_ms(self.total_time_stopwatch.elapsed());
            return Ok(spawn_result_builder
                .set_status(Status::ExecutionDenied)
                .set_exit_code(LOCAL_EXEC_ERROR)
                .set_failure_detail(make_failure_detail(
                    LOCAL_EXEC_ERROR,
                    Status::ExecutionDenied,
                    &action_type,
                ))
                .set_spawn_metrics(self.spawn_metrics.clone())
                .build());
        }

        self.spawn_metrics.input_files =
            spawn.get_input_files().memoized_flatten_and_get_size() as i64;
        let setup_time_stopwatch = Instant::now();
        let mut runfiles_trees: Vec<Arc<dyn RunfilesTree>> = Vec::new();

        for input in spawn.get_input_files().to_list().iter() {
            if let Some(virtual_action_input) = input.as_virtual_action_input() {
                virtual_action_input
                    .atomically_write_relative_to(exec_root)
                    .map_err(|e| exec_io_exception(&e))?;
            } else if input.as_artifact().is_some_and(|a| a.is_runfiles_tree()) {
                let runfiles = context
                    .get_input_metadata_provider()
                    .get_runfiles_metadata(input)
                    .unwrap_or_else(|| {
                        panic!("NullPointerException: no runfiles metadata for {input}")
                    });
                runfiles_trees.push(runfiles.get_runfiles_tree().clone());
            }
        }

        // PORT: profiling dropped.
        runner.update_runfiles(&runfiles_trees);

        let tmp_dir = runner
            .create_action_temp(exec_root)
            .map_err(|e| exec_io_exception(&e))?;
        let result = self.run_in_tmp_dir(
            &tmp_dir,
            spawn_result_builder,
            &action_type,
            setup_time_stopwatch,
        );
        // Delete the temp directory tree, so the next action that this thread executes will get a
        // fresh, empty temp directory.
        if tmp_dir.delete_tree().is_err() {
            // We can't handle this exception in any meaningful way, nor should we, but let's log
            // it.
            // PORT: logging dropped ("failed to delete temp directory ...").
        }
        result
    }

    // Java: com/google/devtools/build/lib/exec/local/LocalSpawnRunner.java:362 LocalSpawnRunner.SubprocessHandler.start
    fn run_in_tmp_dir(
        &mut self,
        tmp_dir: &Path,
        mut spawn_result_builder: crate::actions::SpawnResultBuilder,
        action_type: &str,
        setup_time_stopwatch: Instant,
    ) -> Result<SpawnResult, ExecException> {
        let runner = self.runner;
        let spawn = self.spawn;
        let context = self.context;
        let exec_root = &runner.exec_root;
        let out_err = context.get_file_out_err();
        let mut statistics_path: Option<Path> = None;

        let command_tmp_dir = tmp_dir.get_relative_str("work");
        command_tmp_dir
            .create_directory()
            .map_err(|e| exec_io_exception(&e))?;
        let environment = runner
            .local_env_provider
            .rewrite_local_env(
                spawn.get_environment(),
                &runner.bin_tools,
                command_tmp_dir.get_path_string(),
            )
            .map_err(|e| exec_io_exception(&e))?;

        let mut subprocess_builder = SubprocessBuilder::new(context.get_client_env());
        subprocess_builder.set_working_directory(Some(JavaFile::new(exec_root.get_path_string())));
        subprocess_builder
            .set_stdout_file(JavaFile::new(out_err.get_output_path().get_path_string()));
        subprocess_builder
            .set_stderr_file(JavaFile::new(out_err.get_error_path().get_path_string()));
        subprocess_builder.set_env(Some(environment));
        let mut args: Vec<String>;
        if let Some(process_wrapper) = &runner.process_wrapper {
            // If the process wrapper is enabled, we use its timeout feature, which first
            // interrupts the subprocess and only kills it after a grace period so that the
            // subprocess can output a stack trace, test log or similar, which is incredibly
            // helpful for debugging.
            let mut command_line_builder =
                process_wrapper.command_line_builder(spawn.get_arguments().to_vec());
            command_line_builder
                .add_execution_info(spawn.get_execution_info())
                .set_timeout(context.get_timeout());
            let stats = tmp_dir.get_relative_str("stats.out");
            command_line_builder.set_statistics_path(stats.as_fragment().clone());
            statistics_path = Some(stats);
            args = command_line_builder.build();
        } else {
            subprocess_builder.set_timeout_millis(context.get_timeout().as_millis() as i64);
            args = spawn.get_arguments().to_vec();
        }
        // SubprocessBuilder does not accept relative paths for the first argument, even though
        // Command does. We sometimes get relative paths here, so we need to handle it.
        let argv0 = JavaFile::new(&args[0]);
        if !argv0.is_absolute() && argv0.get_parent().is_some() {
            args[0] =
                JavaFile::new_child(Some(&JavaFile::new(exec_root.get_path_string())), &args[0])
                    .get_absolute_path();
        }
        subprocess_builder.set_argv(args);
        self.spawn_metrics.setup_time_in_ms += to_ms(setup_time_stopwatch.elapsed());

        spawn_result_builder.set_start_time(SystemTime::now());
        let execution_stopwatch = Instant::now();
        // PORT: profiling dropped.
        self.need_cleanup = true;
        let termination_status = match subprocess_builder.start() {
            Ok(subprocess) => {
                // subprocess.getOutputStream().close()
                drop(subprocess.get_output_stream());
                if subprocess.wait_for().is_err() {
                    subprocess.destroy_and_wait();
                    return Err(ExecException::Interrupted);
                }
                let termination_status =
                    TerminationStatus::new(subprocess.exit_value(), subprocess.timedout());
                if blaze_shell::thread_interrupt::interrupted() {
                    // PORT: logging dropped ("Interrupted but didn't throw; status ...").
                    return Err(ExecException::Interrupted);
                }
                termination_status
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {
                return Err(ExecException::Interrupted);
            }
            Err(e) => {
                let msg = e.to_string();
                out_err
                    .write_err(
                        format!("Action failed to execute: java.io.IOException: {msg}\n")
                            .as_bytes(),
                    )
                    .map_err(|e| exec_io_exception(&e))?;
                self.spawn_metrics.total_time_in_ms = to_ms(self.total_time_stopwatch.elapsed());
                return Ok(spawn_result_builder
                    .set_status(Status::ExecutionFailed)
                    .set_exit_code(LOCAL_EXEC_ERROR)
                    .set_failure_detail(make_failure_detail(
                        LOCAL_EXEC_ERROR,
                        Status::ExecutionFailed,
                        action_type,
                    ))
                    .set_spawn_metrics(self.spawn_metrics.clone())
                    .build());
            }
        };
        // TODO(b/62588075): Calculate wall time inside commands instead?
        let wall_time = execution_stopwatch.elapsed();
        self.spawn_metrics.execution_wall_time_in_ms = to_ms(wall_time);

        let was_timeout = termination_status.timed_out()
            || (runner.process_wrapper.is_some()
                && SubprocessHandler::was_timeout(context.get_timeout(), wall_time));
        let exit_code = if was_timeout {
            POSIX_TIMEOUT_EXIT_CODE
        } else {
            termination_status.get_raw_exit_code()
        };
        let status = if was_timeout {
            Status::Timeout
        } else if exit_code == 0 {
            Status::Success
        } else {
            Status::NonZeroExit
        };
        spawn_result_builder
            .set_status(status)
            .set_exit_code(exit_code)
            .set_wall_time_in_ms(wall_time.as_millis() as i32);
        if status != Status::Success {
            spawn_result_builder.set_failure_detail(make_failure_detail(
                exit_code,
                status,
                action_type,
            ));
        }
        if let Some(statistics_path) = &statistics_path {
            spawn_result_builder
                .set_resource_usage_from_proto(statistics_path)
                .map_err(|e| exec_io_exception(&e))?;
        }
        self.spawn_metrics.total_time_in_ms = to_ms(self.total_time_stopwatch.elapsed());
        spawn_result_builder.set_spawn_metrics(self.spawn_metrics.clone());
        Ok(spawn_result_builder.build())
    }

    // Java: com/google/devtools/build/lib/exec/local/LocalSpawnRunner.java:486 LocalSpawnRunner.SubprocessHandler.wasTimeout
    fn was_timeout(timeout: Duration, wall_time: Duration) -> bool {
        !timeout.is_zero() && wall_time > timeout
    }

    /// Clean up any known side-effects that the running spawn may have had on the output tree.
    ///
    /// This is supposed to leave the output tree as it was right after the action executor
    /// created the output directories for the spawn, which means that any outputs have to be
    /// deleted but any top-level directory for tree artifacts has to be kept behind (and empty).
    // Java: com/google/devtools/build/lib/exec/local/LocalSpawnRunner.java:498 LocalSpawnRunner.SubprocessHandler.maybeCleanupOnInterrupt
    fn maybe_cleanup_on_interrupt(&self) {
        if !self.runner.local_execution_options.local_lockfree_output {
            // If we don't allow lockfree executions of local subprocesses, there is no need to
            // clean up anything: we would have already locked the output tree upfront, so we "own"
            // it.
            return;
        }
        if !self.need_cleanup {
            // If the subprocess has not yet started, there is no need to worry about checking
            // on-disk state.
            return;
        }

        for output in self.spawn.get_output_files() {
            // PORT: context.getPathResolver() is ArtifactPathResolver.IDENTITY outside of action
            // file systems: an output's path is the exec root joined with its exec path.
            let path = self.runner.exec_root.get_relative(output.get_exec_path());
            if path.exists() {
                // PORT: logging dropped ("Clearing output ... after interrupt").
                let _ = if output.as_artifact().is_some_and(|a| a.is_tree_artifact()) {
                    path.delete_trees_below()
                } else {
                    path.delete_tree()
                };
                // PORT: logging dropped ("Cannot delete local output ... after interrupt").
            }
        }
    }
}

// Java: com/google/devtools/build/lib/exec/local/LocalSpawnRunner.java:528 LocalSpawnRunner.makeFailureDetail
fn make_failure_detail(exit_code: i32, status: Status, action_type: &str) -> FailureDetail {
    let mut spawn_failure = failure_details::Spawn::default();
    match status {
        Status::Success => {
            panic!("AssertionError: makeFailureDetail() called with Status == SUCCESS")
        }
        Status::NonZeroExit => {
            spawn_failure.code = Code::NonZeroExit as i32;
            spawn_failure.spawn_exit_code = exit_code;
        }
        Status::Timeout => spawn_failure.code = Code::Timeout as i32,
        Status::OutOfMemory => spawn_failure.code = Code::OutOfMemory as i32,
        Status::ExecutionFailed => spawn_failure.code = Code::ExecutionFailed as i32,
        Status::ExecutionFailedCatastrophically => {
            spawn_failure.code = Code::ExecutionFailed as i32;
            spawn_failure.catastrophic = true;
        }
        Status::ExecutionDenied => spawn_failure.code = Code::ExecutionDenied as i32,
        Status::ExecutionDeniedCatastrophically => {
            spawn_failure.code = Code::ExecutionDenied as i32;
            spawn_failure.catastrophic = true;
        }
        Status::RemoteCacheFailed => spawn_failure.code = Code::RemoteCacheFailed as i32,
    }
    FailureDetail {
        message: format!("local spawn failed for {action_type}"),
        category: Some(failure_detail::Category::Spawn(spawn_failure)),
    }
}

/// `NetUtil.getCachedShortHostName()`: `InetAddress.getLocalHost().getHostName()`, i.e. the name
/// from `gethostname(2)` if the name service can resolve it, else "unknown".
// Java: com/google/devtools/build/lib/util/NetUtil.java:39 NetUtil.getCachedShortHostName
pub fn get_cached_short_host_name() -> String {
    static HOSTNAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    HOSTNAME.get_or_init(compute_short_host_name).clone()
}

// Java: com/google/devtools/build/lib/util/NetUtil.java:67 NetUtil.computeShortHostName
fn compute_short_host_name() -> String {
    let mut buf = [0u8; 256];
    // SAFETY: gethostname writes at most buf.len() bytes.
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) };
    if rc != 0 {
        return "unknown".to_string();
    }
    let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    let local = String::from_utf8_lossy(&buf[..len]).into_owned();
    // InetAddress.getLocalHost() resolves the name and throws UnknownHostException when it
    // cannot ("localhost" is special-cased to the loopback address).
    if local == "localhost" {
        return local;
    }
    use std::net::ToSocketAddrs;
    match (local.as_str(), 0u16).to_socket_addrs() {
        Ok(_) => local,
        Err(_) => "unknown".to_string(),
    }
}
