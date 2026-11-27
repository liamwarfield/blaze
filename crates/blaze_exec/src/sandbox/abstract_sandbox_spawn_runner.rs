//! Port of `lib/sandbox/AbstractSandboxSpawnRunner.java`.

use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use blaze_events::{Event, EventHandler, EventKind};
use blaze_optdefs::misc::sandbox_options::SandboxOptions;
use blaze_protos::failure_details::{FailureDetail, failure_detail, sandbox::Code, spawn};
use blaze_shell::{
    CommandFailureUtils, DescribableExecutionUnit, JavaFile, SubprocessBuilder, TerminationStatus,
};
use blaze_util::profiler::{Profiler, ProfilerTask};
use blaze_vfs::{FileSystemExt, Path};
use indexmap::{IndexMap, IndexSet};

use super::sandbox_helpers::{self, interrupted_error, is_interrupted_error};
use super::sandboxed_spawn::SandboxedSpawn;
use crate::actions::resource_manager::{ResourceManager, ResourcePriority};
use crate::actions::{
    EnvironmentalExecException, ExecException, POSIX_TIMEOUT_EXIT_CODE, Spawn, SpawnDescribable,
    SpawnResult, SpawnResultBuilder, Status, UserExecException, spawns,
};
use crate::exec::bin_tools::BinTools;
use crate::exec::exec_command_environment::ExecCommandEnvironment;
use crate::exec::spawn_runner::{ProgressStatus, SpawnExecutionContext};
use crate::exec::tree_deleter::TreeDeleter;

// Java: com/google/devtools/build/lib/sandbox/AbstractSandboxSpawnRunner.java:68 AbstractSandboxSpawnRunner.LOCAL_EXEC_ERROR
const LOCAL_EXEC_ERROR: i32 = -1;

/// `AbstractSandboxSpawnRunner.SANDBOX_DEBUG_SUGGESTION`.
// Java: com/google/devtools/build/lib/sandbox/AbstractSandboxSpawnRunner.java:70 AbstractSandboxSpawnRunner.SANDBOX_DEBUG_SUGGESTION
pub const SANDBOX_DEBUG_SUGGESTION: &str = "\n\nUse --sandbox_debug to see verbose messages from the sandbox and retain the sandbox build root for debugging";

/// What `prepareSpawn` and the steps of `runSpawn` throw: `IOException` or `ExecException`
/// (`InterruptedException` is `ExecException::Interrupted`, or an `io::Error` made by
/// [`interrupted_error`]).
#[derive(Debug)]
pub enum SandboxRunnerError {
    Io(io::Error),
    Exec(ExecException),
}

impl From<io::Error> for SandboxRunnerError {
    fn from(e: io::Error) -> Self {
        SandboxRunnerError::Io(e)
    }
}

impl From<ExecException> for SandboxRunnerError {
    fn from(e: ExecException) -> Self {
        SandboxRunnerError::Exec(e)
    }
}

impl From<UserExecException> for SandboxRunnerError {
    fn from(e: UserExecException) -> Self {
        SandboxRunnerError::Exec(e.into())
    }
}

/// The abstract methods of `AbstractSandboxSpawnRunner` that concrete runners provide.
// Java: com/google/devtools/build/lib/sandbox/AbstractSandboxSpawnRunner.java:67 AbstractSandboxSpawnRunner
pub trait SandboxSpawnRunnerHooks: Send + Sync {
    /// `getName()`.
    fn runner_name(&self) -> String;

    /// `prepareSpawn(spawn, context)`.
    fn prepare_spawn(
        &self,
        spawn: &dyn Spawn,
        context: &dyn SpawnExecutionContext,
    ) -> Result<Box<dyn SandboxedSpawn>, SandboxRunnerError>;

    /// Override this method if you need to run a post condition after the action has executed.
    // Java: com/google/devtools/build/lib/sandbox/AbstractSandboxSpawnRunner.java:182 AbstractSandboxSpawnRunner.verifyPostCondition
    fn verify_post_condition(
        &self,
        _original_spawn: &dyn Spawn,
        _sandbox: &dyn SandboxedSpawn,
        _context: &dyn SpawnExecutionContext,
    ) -> io::Result<()> {
        Ok(())
    }
}

/// Abstract common ancestor for sandbox spawn runners implementing the common parts.
// Java: com/google/devtools/build/lib/sandbox/AbstractSandboxSpawnRunner.java:67 AbstractSandboxSpawnRunner
pub struct AbstractSandboxSpawnRunner {
    sandbox_options: Arc<SandboxOptions>,
    verbose_failures: bool,
    inaccessible_paths: Vec<Path>,
    pub bin_tools: Arc<BinTools>,
    exec_root: Path,
    resource_manager: Arc<ResourceManager>,
    reporter: Arc<dyn EventHandler>,
    pub client_env: IndexMap<String, String>,
}

impl AbstractSandboxSpawnRunner {
    // Java: com/google/devtools/build/lib/sandbox/AbstractSandboxSpawnRunner.java:83 AbstractSandboxSpawnRunner.<init>
    pub fn new(cmd_env: &ExecCommandEnvironment) -> AbstractSandboxSpawnRunner {
        let sandbox_options = cmd_env.sandbox_options.clone();
        let inaccessible_paths = sandbox_options.get_inaccessible_paths(&cmd_env.file_system);
        AbstractSandboxSpawnRunner {
            verbose_failures: cmd_env.execution_options.verbose_failures,
            inaccessible_paths,
            sandbox_options,
            bin_tools: cmd_env.bin_tools.clone(),
            exec_root: cmd_env.exec_root.clone(),
            resource_manager: cmd_env.resource_manager.clone(),
            reporter: cmd_env.reporter.clone(),
            client_env: cmd_env.client_env.clone(),
        }
    }

    /// `exec(spawn, context)`.
    // Java: com/google/devtools/build/lib/sandbox/AbstractSandboxSpawnRunner.java:96 AbstractSandboxSpawnRunner.exec
    pub fn exec(
        &self,
        hooks: &dyn SandboxSpawnRunnerHooks,
        spawn: &dyn Spawn,
        context: &dyn SpawnExecutionContext,
    ) -> Result<SpawnResult, ExecException> {
        let owner = spawn.get_resource_owner();
        context.report(ProgressStatus::Scheduling(hooks.runner_name()));

        let result: Result<SpawnResult, SandboxRunnerError> = (|| {
            // Java: com/google/devtools/build/lib/sandbox/AbstractSandboxSpawnRunner.java:102 AbstractSandboxSpawnRunner.exec
            {
                let _c = Profiler::instance().profile(ProfilerTask::INFO, "context.prefetchInputs");
                context.prefetch_inputs_and_wait()?;
            }

            let local_resources = spawn.get_local_resources()?;
            let _ignored = self.resource_manager.acquire_resources(
                owner,
                &local_resources,
                if context.speculating() {
                    ResourcePriority::DynamicStandalone
                } else {
                    ResourcePriority::Local
                },
            )?;
            context.report(ProgressStatus::Executing(hooks.runner_name()));
            let sandbox = hooks.prepare_spawn(spawn, context)?;
            self.run_spawn(hooks, spawn, sandbox, context)
        })();
        match result {
            Ok(r) => Ok(r),
            Err(SandboxRunnerError::Exec(e)) => Err(e),
            Err(SandboxRunnerError::Io(e)) if is_interrupted_error(&e) => {
                Err(ExecException::Interrupted)
            }
            Err(SandboxRunnerError::Io(e)) => {
                let failure_detail = sandbox_helpers::create_failure_detail(
                    "I/O exception during sandboxed execution",
                    Code::ExecutionIoException,
                );
                Err(UserExecException::with_cause(&e, failure_detail).into())
            }
        }
    }

    /// `canExec(spawn)`.
    // Java: com/google/devtools/build/lib/sandbox/AbstractSandboxSpawnRunner.java:126 AbstractSandboxSpawnRunner.canExec
    pub fn can_exec(spawn: &dyn Spawn) -> bool {
        spawns::may_be_sandboxed(spawn)
    }

    // Java: com/google/devtools/build/lib/sandbox/AbstractSandboxSpawnRunner.java:138 AbstractSandboxSpawnRunner.runSpawn
    fn run_spawn(
        &self,
        hooks: &dyn SandboxSpawnRunnerHooks,
        original_spawn: &dyn Spawn,
        sandbox: Box<dyn SandboxedSpawn>,
        context: &dyn SpawnExecutionContext,
    ) -> Result<SpawnResult, SandboxRunnerError> {
        let result: Result<SpawnResult, SandboxRunnerError> = (|| {
            // Java: com/google/devtools/build/lib/sandbox/AbstractSandboxSpawnRunner.java:142 AbstractSandboxSpawnRunner.runSpawn
            let created = {
                let _c =
                    Profiler::instance().profile(ProfilerTask::INFO, "sandbox.createFileSystem");
                sandbox.create_file_system()
            };
            if let Err(e) = created {
                if is_interrupted_error(&e) {
                    return Err(ExecException::Interrupted.into());
                }
                let failure_detail = sandbox_helpers::create_failure_detail(
                    "Could not copy inputs into sandbox",
                    Code::CopyInputsIoException,
                );
                return Err(ExecException::from(EnvironmentalExecException::with_cause(
                    &e,
                    failure_detail,
                ))
                .into());
            }
            // Java: com/google/devtools/build/lib/sandbox/AbstractSandboxSpawnRunner.java:151 AbstractSandboxSpawnRunner.runSpawn
            let result = {
                let _c = Profiler::instance().profile(ProfilerTask::INFO, "subprocess.run");
                self.run(hooks, original_spawn, &*sandbox, context)?
            };
            // Java: com/google/devtools/build/lib/sandbox/AbstractSandboxSpawnRunner.java:154 AbstractSandboxSpawnRunner.runSpawn
            {
                let _c =
                    Profiler::instance().profile(ProfilerTask::INFO, "sandbox.verifyPostCondition");
                hooks.verify_post_condition(original_spawn, &*sandbox, context)?;
            }

            context.lock_output_files(
                result.exit_code(),
                result
                    .failure_detail()
                    .map(|d| d.message.as_str())
                    .unwrap_or(""),
                Some(context.get_file_out_err()),
            )?;
            // Java: com/google/devtools/build/lib/sandbox/AbstractSandboxSpawnRunner.java:162 AbstractSandboxSpawnRunner.runSpawn
            let copied = {
                let _c = Profiler::instance().profile(ProfilerTask::INFO, "sandbox.copyOutputs");
                // We copy the outputs even when the command failed.
                sandbox.copy_outputs(&self.exec_root)
            };
            if let Err(e) = copied {
                if is_interrupted_error(&e) {
                    return Err(ExecException::Interrupted.into());
                }
                let failure_detail = sandbox_helpers::create_failure_detail(
                    "Could not copy outputs from sandbox",
                    Code::CopyOutputsIoException,
                );
                return Err(ExecException::from(EnvironmentalExecException::with_cause(
                    &e,
                    failure_detail,
                ))
                .into());
            }
            Ok(result)
        })();
        // finally
        if !self.sandbox_options.sandbox_debug {
            // Java: com/google/devtools/build/lib/sandbox/AbstractSandboxSpawnRunner.java:174 AbstractSandboxSpawnRunner.runSpawn
            let _c = Profiler::instance().profile(ProfilerTask::INFO, "sandbox.delete");
            sandbox.delete();
        }
        result
    }

    // Java: com/google/devtools/build/lib/sandbox/AbstractSandboxSpawnRunner.java:186 AbstractSandboxSpawnRunner.makeFailureMessage
    fn make_failure_message(
        &self,
        original_spawn: &dyn Spawn,
        sandbox: &dyn SandboxedSpawn,
    ) -> String {
        if self.sandbox_options.sandbox_debug {
            CommandFailureUtils::describe_command_failure(
                true,
                Some(sandbox.get_sandbox_exec_root().get_path_string()),
                sandbox as &dyn DescribableExecutionUnit,
            )
        } else {
            CommandFailureUtils::describe_command_failure(
                self.verbose_failures,
                Some(sandbox.get_sandbox_exec_root().get_path_string()),
                &SpawnDescribable(original_spawn),
            ) + SANDBOX_DEBUG_SUGGESTION
        }
    }

    // Java: com/google/devtools/build/lib/sandbox/AbstractSandboxSpawnRunner.java:197 AbstractSandboxSpawnRunner.run
    fn run(
        &self,
        hooks: &dyn SandboxSpawnRunnerHooks,
        original_spawn: &dyn Spawn,
        sandbox: &dyn SandboxedSpawn,
        context: &dyn SpawnExecutionContext,
    ) -> io::Result<SpawnResult> {
        // Java: SpawnRunner.getSpawnResultBuilder(context).
        let mut spawn_result_builder = SpawnResultBuilder::new();
        spawn_result_builder
            .set_runner_name(&hooks.runner_name())
            .set_digest(context.get_digest());

        let out_err = context.get_file_out_err();
        let timeout = context.get_timeout();

        let mut subprocess_builder = SubprocessBuilder::new(&self.client_env);
        subprocess_builder.set_working_directory(Some(JavaFile::new(
            sandbox.get_sandbox_exec_root().get_path_string(),
        )));
        subprocess_builder
            .set_stdout_file(JavaFile::new(out_err.get_output_path().get_path_string()));
        subprocess_builder
            .set_stderr_file(JavaFile::new(out_err.get_error_path().get_path_string()));
        subprocess_builder.set_env(sandbox.get_environment());
        subprocess_builder.set_argv(sandbox.get_arguments());
        let use_subprocess_timeout = sandbox.use_subprocess_timeout();
        if use_subprocess_timeout {
            subprocess_builder.set_timeout_millis(timeout.to_millis());
        }
        let start_time = SystemTime::now();
        let start_instant = Instant::now();
        let termination_status = match subprocess_builder.start() {
            Ok(subprocess) => {
                drop(subprocess.get_output_stream());
                match subprocess.wait_for() {
                    Ok(()) => {
                        TerminationStatus::new(subprocess.exit_value(), subprocess.timedout())
                    }
                    Err(_interrupted) => {
                        subprocess.destroy_and_wait();
                        return Err(interrupted_error(None));
                    }
                }
            }
            Err(e) => {
                let exception_msg = e.to_string();
                let sandbox_debug_output = get_sandbox_debug_output(sandbox)?;

                let mut msg = String::from("Action failed to execute: java.io.IOException: ");
                msg.push_str(&exception_msg);
                msg.push('\n');
                if !sandbox_debug_output.is_empty() {
                    msg.push_str("Sandbox debug output:\n");
                    msg.push_str(&sandbox_debug_output);
                    msg.push('\n');
                }

                out_err.write_err(msg.as_bytes())?;
                let message = self.make_failure_message(original_spawn, sandbox);
                return Ok(spawn_result_builder
                    .set_status(Status::ExecutionFailed)
                    .set_exit_code(LOCAL_EXEC_ERROR)
                    .set_failure_message(&message)
                    .set_failure_detail(sandbox_helpers::create_failure_detail(
                        &message,
                        Code::SubprocessStartFailed,
                    ))
                    .build());
            }
        };

        // TODO(b/62588075): Calculate wall time inside Subprocess instead?
        let wall_time = start_instant.elapsed();
        let was_timeout = (use_subprocess_timeout && termination_status.timed_out())
            || (!use_subprocess_timeout && was_timeout(timeout, wall_time));

        let exit_code;
        let status;
        let failure_message;
        let failure_detail: Option<FailureDetail>;
        if was_timeout {
            exit_code = POSIX_TIMEOUT_EXIT_CODE;
            status = Status::Timeout;
            failure_message = self.make_failure_message(original_spawn, sandbox);
            failure_detail = Some(FailureDetail {
                message: failure_message.clone(),
                category: Some(failure_detail::Category::Spawn(
                    blaze_protos::failure_details::Spawn {
                        code: spawn::Code::Timeout as i32,
                        ..Default::default()
                    },
                )),
            });
        } else {
            exit_code = termination_status.get_raw_exit_code();
            if exit_code == 0 {
                status = Status::Success;
                failure_message = String::new();
                failure_detail = None;
            } else {
                status = Status::NonZeroExit;
                failure_message = self.make_failure_message(original_spawn, sandbox);
                failure_detail = Some(FailureDetail {
                    message: failure_message.clone(),
                    category: Some(failure_detail::Category::Spawn(
                        blaze_protos::failure_details::Spawn {
                            code: spawn::Code::NonZeroExit as i32,
                            spawn_exit_code: exit_code,
                            ..Default::default()
                        },
                    )),
                });
            }
        }

        spawn_result_builder
            .set_status(status)
            .set_exit_code(exit_code)
            .set_start_time(start_time)
            .set_wall_time_in_ms(wall_time.as_millis() as i32)
            .set_failure_message(&failure_message);

        if let Some(failure_detail) = failure_detail {
            spawn_result_builder.set_failure_detail(failure_detail);
        }

        let sandbox_debug_output = get_sandbox_debug_output(sandbox)?;
        if !sandbox_debug_output.is_empty() {
            self.reporter.handle(&Event::of(
                EventKind::Debug,
                format!(
                    "Sandbox debug output for {} {}:\n{}",
                    original_spawn.get_mnemonic(),
                    original_spawn
                        .get_target_label()
                        .map(|l| l.to_string())
                        .unwrap_or_else(|| "null".to_string()),
                    sandbox_debug_output
                ),
            ));
        }

        if let Some(statistics_path) = sandbox.get_statistics_path() {
            spawn_result_builder.set_resource_usage_from_proto(statistics_path)?;
        }

        Ok(spawn_result_builder.build())
    }

    /// Gets the list of directories that the spawn will assume to be writable.
    ///
    /// `sandbox_exec_root`: the exec root of the sandbox; `env`: the environment of the sandboxed
    /// processes. Fails because we might resolve symlinks.
    // Java: com/google/devtools/build/lib/sandbox/AbstractSandboxSpawnRunner.java:352 AbstractSandboxSpawnRunner.getWritableDirs
    pub fn get_writable_dirs(
        &self,
        sandbox_exec_root: &Path,
        env: &IndexMap<String, String>,
    ) -> io::Result<IndexSet<Path>> {
        // We have to make the TEST_TMPDIR directory writable if it is specified.
        let mut writable_paths: IndexSet<Path> = IndexSet::new();

        // On Windows, sandboxExecRoot is actually the main execroot. We will specify
        // exactly which output path is writable.
        // PORT: skipped, windows only (the OS check).
        writable_paths.insert(sandbox_exec_root.clone());

        if let Some(test_tmpdir) = env.get("TEST_TMPDIR") {
            add_writable_path(
                sandbox_exec_root,
                &mut writable_paths,
                test_tmpdir,
                "Cannot resolve symlinks in TEST_TMPDIR because it doesn't exist: \"%s\"",
            )?;
        }
        // As of 2019-07-08:
        // - every caller of `getWritableDirs` passes a LocalEnvProvider-processed environment as
        //   `env`, therefore `env` surely has an entry for TMPDIR on Unix and TEMP/TMP on Windows.
        // PORT: skipped, windows only (TEMP/TMP).
        add_writable_path(
            sandbox_exec_root,
            &mut writable_paths,
            env.get("TMPDIR").expect("NullPointerException: TMPDIR"),
            "Cannot resolve symlinks in TMPDIR because it doesn't exist: \"%s\"",
        )?;

        let file_system = sandbox_exec_root.get_file_system();
        for writable_path in &self.sandbox_options.sandbox_writable_path {
            let path = file_system.get_path(writable_path);
            writable_paths.insert(path.clone());
            // TODO(laszlocsomor): Remove if guard when path.resolveSymbolicLinks supports
            // non-symlink
            writable_paths.insert(path.resolve_symbolic_links()?);
        }

        Ok(writable_paths)
    }

    /// `getInaccessiblePaths()`.
    pub fn get_inaccessible_paths(&self) -> &[Path] {
        &self.inaccessible_paths
    }

    /// `getSandboxOptions()`.
    pub fn get_sandbox_options(&self) -> &SandboxOptions {
        &self.sandbox_options
    }

    /// `cleanupSandboxBase(sandboxBase, treeDeleter)`, for the runner named `name`.
    // Java: com/google/devtools/build/lib/sandbox/AbstractSandboxSpawnRunner.java:445 AbstractSandboxSpawnRunner.cleanupSandboxBase
    pub fn cleanup_sandbox_base(
        &self,
        name: &str,
        sandbox_base: &Path,
        tree_deleter: &dyn TreeDeleter,
    ) -> io::Result<()> {
        let root = sandbox_base.get_child(name);
        if root.exists() {
            for child in root.get_directory_entries()? {
                tree_deleter.delete_tree(&child)?;
            }
        }
        Ok(())
    }
}

// Java: com/google/devtools/build/lib/sandbox/AbstractSandboxSpawnRunner.java:324 AbstractSandboxSpawnRunner.getSandboxDebugOutput
fn get_sandbox_debug_output(sandbox: &dyn SandboxedSpawn) -> io::Result<String> {
    let mut sandbox_debug_output: Option<String> = None;
    if let Some(sandbox_debug_path) = sandbox.get_sandbox_debug_path()
        && sandbox_debug_path.exists()
    {
        let mut input_stream = sandbox_debug_path.get_input_stream()?;
        let mut bytes = Vec::new();
        io::Read::read_to_end(&mut input_stream, &mut bytes)?;
        // `new String(readAllBytes(), UTF_8)`
        let msg = blaze_util::string_encoding::java_new_string_utf8(&bytes);
        if !msg.is_empty() {
            sandbox_debug_output = Some(msg);
        }
    }
    let interactive_debug_instructions = sandbox.get_interactive_debug_instructions();
    Ok([sandbox_debug_output, interactive_debug_instructions]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("\n"))
}

// Java: com/google/devtools/build/lib/sandbox/AbstractSandboxSpawnRunner.java:341 AbstractSandboxSpawnRunner.wasTimeout
fn was_timeout(timeout: blaze_util::java_duration::JavaDuration, wall_time: Duration) -> bool {
    // wallTime.compareTo(timeout) > 0: a negative timeout is always exceeded.
    !timeout.is_zero() && blaze_util::java_duration::JavaDuration::from(wall_time) > timeout
}

// Java: com/google/devtools/build/lib/sandbox/AbstractSandboxSpawnRunner.java:407 AbstractSandboxSpawnRunner.addWritablePath
fn add_writable_path(
    sandbox_exec_root: &Path,
    writable_paths: &mut IndexSet<Path>,
    path_string: &str,
    path_does_not_exist_error_template: &str,
) -> io::Result<()> {
    let path = sandbox_exec_root.get_relative_str(path_string);
    if path.starts_with(sandbox_exec_root) {
        // We add this path even though it is below sandboxExecRoot (and thus already writable as
        // a subpath) to take advantage of the side-effect that SymlinkedExecRoot also creates this
        // needed directory if it doesn't exist yet.
        writable_paths.insert(path);
    } else if path.exists() {
        // If `path` itself is a symlink, then adding it to `writablePaths` would result in making
        // the symlink itself writable, not what it points to. Therefore we need to resolve
        // symlinks in `path`, however for that we need `path` to exist.
        writable_paths.insert(path.resolve_symbolic_links()?);
    } else {
        return Err(io::Error::other(
            path_does_not_exist_error_template.replace("%s", path.get_path_string()),
        ));
    }
    Ok(())
}
