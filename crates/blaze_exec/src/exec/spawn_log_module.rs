//! Port of `lib/bazel/SpawnLogModule.java`: on-demand spawn logging, driven by
//! `--execution_log_{json,binary,compact}_file` and `--execution_log_sort`.

use std::fmt;
use std::io;
use std::sync::Arc;

use blaze_events::{Event, EventHandler};
use blaze_optdefs::command::execution_options::ExecutionOptions;
use blaze_optdefs::misc::remote_options::RemoteOptions;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail};
use blaze_vfs::{FileSystemExt, FileSystemRef, Path, PathFragment};

use super::compact_spawn_log_context::CompactSpawnLogContext;
use super::expanded_spawn_log_context::{Encoding, ExpandedSpawnLogContext};
use super::spawn_log_context::SpawnLogContext;

// Java: com/google/devtools/build/lib/bazel/SpawnLogModule.java:49 SpawnLogModule.EXEC_LOG_FILENAME
pub const EXEC_LOG_FILENAME: &str = "execution_log.binpb.zst";

/// The parts of `CommandEnvironment` that `SpawnLogModule` reads.
// Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java CommandEnvironment
pub struct SpawnLogModuleEnv<'a> {
    /// `getOptions().getOptions(ExecutionOptions.class)`.
    pub execution_options: Option<&'a ExecutionOptions>,
    /// `getOptions().getOptions(RemoteOptions.class)`.
    pub remote_options: Option<Arc<RemoteOptions>>,
    /// `getOptions().getOptions(BuildLanguageOptions.class).experimentalSiblingRepositoryLayout`.
    pub sibling_repository_layout: bool,
    /// `getRuntime().getFileSystem()`.
    pub file_system: &'a FileSystemRef,
    /// `getOutputBase()`.
    pub output_base: &'a Path,
    /// `getExecRoot()`.
    pub exec_root: &'a Path,
    /// `getWorkspace()`.
    pub workspace: Option<&'a Path>,
    /// `getWorkingDirectory()`.
    pub working_directory: &'a Path,
    /// `getWorkspaceName()`.
    pub workspace_name: &'a str,
    /// `getCommandId().toString()`.
    pub command_id: &'a str,
    /// `getReporter()`.
    pub reporter: Arc<dyn EventHandler>,
}

/// An `AbruptExitException` carrying `DetailedExitCode.of(failureDetail)`, as handed to
/// `BlazeModuleEnvironment.exit` or thrown from `afterCommand`.
// Java: com/google/devtools/build/lib/util/AbruptExitException.java AbruptExitException
#[derive(Clone, Debug)]
pub struct SpawnLogAbruptExit {
    /// The failure detail (its `message` is the exception message).
    pub failure_detail: FailureDetail,
}

impl fmt::Display for SpawnLogAbruptExit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.failure_detail.message)
    }
}

impl std::error::Error for SpawnLogAbruptExit {}

/// Module providing on-demand spawn logging.
// Java: com/google/devtools/build/lib/bazel/SpawnLogModule.java:48 SpawnLogModule
#[derive(Default)]
pub struct SpawnLogModule {
    spawn_log_context: Option<Arc<dyn SpawnLogContext>>,
    output_path: Option<Path>,
    abrupt_exit: Option<SpawnLogAbruptExit>,
}

impl SpawnLogModule {
    pub fn new() -> SpawnLogModule {
        SpawnLogModule::default()
    }

    // Java: com/google/devtools/build/lib/bazel/SpawnLogModule.java:56 SpawnLogModule.clear
    fn clear(&mut self) {
        self.spawn_log_context = None;
        self.output_path = None;
        self.abrupt_exit = None;
    }

    /// Returns `Err` with the exit that Java requests through
    /// `env.getBlazeModuleEnvironment().exit(...)`.
    // Java: com/google/devtools/build/lib/bazel/SpawnLogModule.java:62 SpawnLogModule.initOutputs
    fn init_outputs(&mut self, env: &SpawnLogModuleEnv<'_>) -> Result<(), InitError> {
        self.clear();

        let Some(execution_options) = env.execution_options else {
            return Ok(());
        };

        let num_formats = [
            execution_options.execution_log_compact_file.is_some(),
            execution_options.execution_log_binary_file.is_some(),
            execution_options.execution_log_json_file.is_some(),
        ]
        .iter()
        .filter(|b| **b)
        .count();

        if num_formats == 0 {
            // No logging requested.
            return Ok(());
        }

        if num_formats > 1 {
            let message = "Must specify at most one of --execution_log_binary_file, \
                           --execution_log_json_file and --execution_log_compact_file";
            return Err(InitError::Exit(SpawnLogAbruptExit {
                failure_detail: FailureDetail {
                    message: message.to_string(),
                    category: Some(failure_detail::Category::ExecutionOptions(
                        failure_details::ExecutionOptions {
                            code: failure_details::execution_options::Code::MultipleExecutionLogFormats
                                as i32,
                        },
                    )),
                },
            }));
        }

        let output_base = env.output_base;
        // PORT: `getOutputServiceAwareXattrProvider` is the file system's fast digest (see
        // `compute_digest`).

        if let Some(compact_file) = &execution_options.execution_log_compact_file {
            let output_path = Self::get_absolute_path(compact_file.get(), env);
            self.output_path = Some(output_path.clone());

            // PORT: the InterruptedException branch ("Error while setting up the execution log:
            // ...") has no Rust counterpart.
            self.spawn_log_context = Some(Arc::new(
                CompactSpawnLogContext::new(
                    &output_path,
                    env.exec_root.as_fragment().clone(),
                    env.workspace_name.to_string(),
                    env.sibling_repository_layout,
                    env.remote_options.clone(),
                    env.file_system.get_digest_function(),
                    env.command_id.to_string(),
                    Arc::clone(&env.reporter),
                )
                .map_err(InitError::Io)?,
            ));
        } else {
            let mut encoding = None;

            if let Some(binary_file) = &execution_options.execution_log_binary_file {
                encoding = Some(Encoding::Binary);
                self.output_path = Some(Self::get_absolute_path(binary_file.get(), env));
            } else if let Some(json_file) = &execution_options.execution_log_json_file {
                encoding = Some(Encoding::Json);
                self.output_path = Some(Self::get_absolute_path(json_file.get(), env));
            }

            // Use a well-known temporary path to avoid accumulation of potentially large files
            // in /tmp due to abnormally terminated invocations (e.g., when running out of
            // memory).
            let temp_path = output_base.get_relative_str(EXEC_LOG_FILENAME);

            self.spawn_log_context = Some(Arc::new(
                ExpandedSpawnLogContext::new(
                    self.output_path
                        .clone()
                        .expect("NullPointerException: outputPath"),
                    temp_path,
                    encoding.expect("NullPointerException: encoding"),
                    /* sorted= */ execution_options.execution_log_sort,
                    env.exec_root.as_fragment().clone(),
                    env.remote_options.clone(),
                    env.file_system.get_digest_function(),
                )
                .map_err(InitError::Io)?,
            ));
        }
        Ok(())
    }

    /// If the given path is absolute path, leave it as it is. If the given path is a relative
    /// path, it is relative to the current working directory. If the given path starts with
    /// '%workspace%, it is relative to the workspace root, which is the output of `bazel info
    /// workspace`.
    // Java: com/google/devtools/build/lib/bazel/SpawnLogModule.java:165 SpawnLogModule.getAbsolutePath
    fn get_absolute_path(path: &PathFragment, env: &SpawnLogModuleEnv<'_>) -> Path {
        let mut path_string = path.get_path_string().to_string();
        if let Some(workspace) = env.workspace {
            path_string = path_string.replace("%workspace%", workspace.get_path_string());
        }
        if !PathFragment::is_absolute_str(&path_string) {
            return env.working_directory.get_relative_str(&path_string);
        }

        env.file_system.get_path(&path_string)
    }

    /// `registerActionContexts`: the context to register as the `SpawnLogContext`, if any.
    // Java: com/google/devtools/build/lib/bazel/SpawnLogModule.java:178 SpawnLogModule.registerActionContexts
    pub fn register_action_contexts(&self) -> Option<Arc<dyn SpawnLogContext>> {
        self.spawn_log_context.clone()
    }

    /// `executorInit`. An `Err` is the abrupt exit Java requests through
    /// `env.getBlazeModuleEnvironment().exit(...)`.
    // Java: com/google/devtools/build/lib/bazel/SpawnLogModule.java:188 SpawnLogModule.executorInit
    pub fn executor_init(&mut self, env: &SpawnLogModuleEnv<'_>) -> Result<(), SpawnLogAbruptExit> {
        // PORT: `env.getEventBus().register(this)`: the caller calls `build_complete`.
        match self.init_outputs(env) {
            Ok(()) => Ok(()),
            Err(InitError::Exit(exit)) => Err(exit),
            Err(InitError::Io(e)) => {
                env.reporter.handle(&Event::error(e.to_string()));
                Err(Self::create_detailed_exit_code(
                    format!("Error initializing execution log: {e}"),
                    failure_details::execution::Code::ExecutionLogInitializationFailure,
                ))
            }
        }
    }

    /// `buildComplete(BuildCompleteEvent)`: finalizes the log. Returns the file to add to the
    /// build tool log collection (`addLocalFile(name, path)`), if the log is to be published.
    // Java: com/google/devtools/build/lib/bazel/SpawnLogModule.java:205 SpawnLogModule.buildComplete
    pub fn build_complete(&mut self) -> Option<(String, Path)> {
        // The log must be finalized in buildComplete() instead of afterCommand(), because it's
        // our last chance to publish it to the build event protocol.

        let Some(spawn_log_context) = self.spawn_log_context.clone() else {
            // No logging requested.
            self.clear();
            return None;
        };

        let mut published = None;
        match spawn_log_context.close() {
            Ok(()) => {
                if spawn_log_context.should_publish() {
                    published = self
                        .output_path
                        .clone()
                        .map(|path| (EXEC_LOG_FILENAME.to_string(), path));
                }
            }
            Err(e) => {
                self.abrupt_exit = Some(Self::create_detailed_exit_code(
                    format!("Error writing execution log: {e}"),
                    failure_details::execution::Code::ExecutionLogWriteFailure,
                ));
            }
        }
        // finally. PORT: as in Java, `clear()` also drops an exit recorded just above, so
        // `afterCommand` does not report it.
        self.clear();
        published
    }

    // Java: com/google/devtools/build/lib/bazel/SpawnLogModule.java:232 SpawnLogModule.createDetailedExitCode
    fn create_detailed_exit_code(
        message: String,
        detailed_code: failure_details::execution::Code,
    ) -> SpawnLogAbruptExit {
        SpawnLogAbruptExit {
            failure_detail: FailureDetail {
                message,
                category: Some(failure_detail::Category::Execution(
                    failure_details::Execution {
                        code: detailed_code as i32,
                    },
                )),
            },
        }
    }

    // Java: com/google/devtools/build/lib/bazel/SpawnLogModule.java:241 SpawnLogModule.afterCommand
    pub fn after_command(&mut self) -> Result<(), SpawnLogAbruptExit> {
        match self.abrupt_exit.take() {
            Some(exit) => Err(exit),
            None => Ok(()),
        }
    }
}

/// The two ways `initOutputs` fails: an `IOException`, or a requested exit.
enum InitError {
    Io(io::Error),
    Exit(SpawnLogAbruptExit),
}

/// Runs `SpawnLogModule.executorInit` for a command and returns the context to register: the
/// constructor of the execution log from `ExecutionOptions` and the command's paths.
pub fn create_spawn_log_context(
    module: &mut SpawnLogModule,
    env: &SpawnLogModuleEnv<'_>,
) -> Result<Option<Arc<dyn SpawnLogContext>>, SpawnLogAbruptExit> {
    module.executor_init(env)?;
    Ok(module.register_action_contexts())
}

/// Whether the command asked for an execution log (`initOutputs` creates a context): one of
/// `--execution_log_compact_file`, `--execution_log_binary_file`, `--execution_log_json_file`.
/// More than one is `initOutputs`' error, which also ends with no context registered.
// Java: com/google/devtools/build/lib/bazel/SpawnLogModule.java:70 SpawnLogModule.initOutputs
pub fn execution_log_requested(execution_options: Option<&ExecutionOptions>) -> bool {
    execution_options.is_some_and(|o| {
        o.execution_log_compact_file.is_some()
            || o.execution_log_binary_file.is_some()
            || o.execution_log_json_file.is_some()
    })
}

/// The `SpawnLogContext` the command's executor registers before `SpawnLogModule.executorInit`
/// has run.
///
/// PORT: Java builds its executor (and runs every module's `executorInit`, which creates the
/// log file) in `new ExecutionTool(env, request)`, after the target patterns are evaluated (and,
/// without merged analysis and execution, after the analysis); a build that fails earlier
/// writes no log. Blaze builds the executor with the command's Skyframe executor, before
/// loading, so it registers this context there, when a log is requested
/// ([`execution_log_requested`]), and [`Self::set`] installs the module's context where Java runs
/// `executorInit`. Until then (no spawn runs before it) the calls do nothing.
#[derive(Default)]
pub struct DeferredSpawnLogContext {
    inner: std::sync::OnceLock<Option<Arc<dyn SpawnLogContext>>>,
}

impl DeferredSpawnLogContext {
    pub fn new() -> DeferredSpawnLogContext {
        DeferredSpawnLogContext::default()
    }

    /// Installs the context `executorInit` created (`registerActionContexts`), once.
    pub fn set(&self, context: Option<Arc<dyn SpawnLogContext>>) {
        blaze_util::check_state!(
            self.inner.set(context).is_ok(),
            "the execution log was initialized twice"
        );
    }

    fn get(&self) -> Option<&Arc<dyn SpawnLogContext>> {
        self.inner.get().and_then(|c| c.as_ref())
    }
}

impl SpawnLogContext for DeferredSpawnLogContext {
    fn log_spawn(
        &self,
        spawn: &dyn crate::actions::Spawn,
        input_metadata_provider: &dyn crate::actions::InputMetadataProvider,
        input_map: &dyn Fn() -> Arc<crate::actions::InputMapping>,
        file_system: &FileSystemRef,
        timeout: std::time::Duration,
        result: &crate::actions::SpawnResult,
    ) -> Result<(), crate::actions::ExecException> {
        match self.get() {
            Some(c) => c.log_spawn(
                spawn,
                input_metadata_provider,
                input_map,
                file_system,
                timeout,
                result,
            ),
            None => Ok(()),
        }
    }

    fn log_symlink_action(
        &self,
        action: &dyn super::spawn_log_context::SpawnLogAction,
    ) -> io::Result<()> {
        match self.get() {
            Some(c) => c.log_symlink_action(action),
            None => Ok(()),
        }
    }

    fn close(&self) -> io::Result<()> {
        match self.get() {
            Some(c) => c.close(),
            None => Ok(()),
        }
    }

    fn should_publish(&self) -> bool {
        self.get().is_some_and(|c| c.should_publish())
    }
}
