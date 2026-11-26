//! The methods shared by `repository_ctx` and `module_ctx`.
//!
//! Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java
//!
//! PORT: the Java class is both the state and the Starlark object. In Blaze the state is
//! [`StarlarkBaseExternalContext`] (inside the thread's [`ExternalContext`] payload) and the
//! Starlark methods are registered on the `repository_ctx` / `module_ctx` handle types by
//! [`base_external_context_methods`]; each native fetches the state with
//! [`ExternalContext::from_eval`]. Methods that need the subclass (`isRemotable`,
//! `shouldDeleteWorkingDirectoryOnClose`) or raise `NeedsSkyframeRestartException` /
//! `InterruptedException` are implemented on [`ExternalContext`].
//!
//! PORT: `WorkspaceRuleEvent`s (Java posts one per call to the listener) are not ported; they
//! are only consumed by `--experimental_workspace_rules_log_file`, which Blaze does not support.
//! Profiler spans are skipped.
#![allow(non_snake_case)]

use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt;
use std::io;
use std::io::Write as _;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;
use std::time::Instant;

use allocative::Allocative;
use base64::Engine as _;
use blaze_cmdline::Label;
use blaze_cmdline::RepositoryName;
use blaze_cmdline::repository_name::MAIN;
use blaze_events::Event;
use blaze_events::FetchProgress;
use blaze_events::Postable;
use blaze_repo_fetch::authandtls::Headers;
use blaze_repo_fetch::cache::KeyType;
use blaze_repo_fetch::cache::download_cache;
use blaze_repo_fetch::decompressor;
use blaze_repo_fetch::decompressor::DecompressorDescriptor;
use blaze_repo_fetch::downloader::Checksum;
use blaze_repo_fetch::downloader::DownloadManager;
use blaze_repo_fetch::downloader::http_utils;
use blaze_repo_fetch::errors::IoException;
use blaze_repo_fetch::uri::JavaUri;
use blaze_shell::JavaFile;
use blaze_sky_loading::directories::LoadingDirectories;
use blaze_sky_loading::io::IOException;
use blaze_starlark::StarlarkSemantics;
use blaze_starlark::label::LabelValue;
use blaze_starlark::structs::StarlarkInfo;
use blaze_starlark::structs::StructProvider;
use blaze_util::string_encoding::latin1_decode;
use blaze_util::string_encoding::latin1_encode;
use blaze_vfs::Path;
use blaze_vfs::PathFragment;
use blaze_vfs::RootedPath;
use indexmap::IndexMap;
use indexmap::IndexSet;
use parking_lot::Mutex;
use starlark::any::ProvidesStaticType;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::StarlarkValue;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::dict::DictRef;
use starlark::values::none::NoneOr;
use starlark::values::none::NoneType;
use starlark::values::starlark_value;

use super::context::EvaluationAbort;
use super::context::ExternalContext;
use super::context::ModuleCtx;
use super::context::RepositoryCtx;
use super::execution_result::StarlarkExecutionResult;
use super::os::StarlarkOS;
use super::path::StarlarkPath;
use crate::env_handle::SkyEnv;
use crate::repository::exceptions::JavaThrowable;
use crate::repository::repo_recorded_input::MaybeValue;
use crate::repository::repo_recorded_input::RepoCacheFriendlyPath;
use crate::repository::repo_recorded_input::RepoRecordedInput;
use crate::repository::repo_recorded_input::RepoRecordedInputWithValue;
use crate::repository::repository_utils;

/// Java `EvalException(msg)` / `Starlark.errorf(...)` thrown by a Starlark method.
///
/// A `RepositoryFunctionException` goes through [`repository_function_exception`] instead: its
/// message is not its cause's message.
pub fn eval_error(msg: impl fmt::Display) -> starlark::Error {
    starlark::Error::new_other(anyhow::anyhow!("{msg}"))
}

/// Java `throw new RepositoryFunctionException(cause, transience)` escaping a `@StarlarkMethod`.
///
/// `RepositoryFunctionException` is a `SkyFunctionException`, whose constructor calls
/// `Exception(Throwable cause)`: its message is `cause.toString()`. The interpreter then wraps
/// the checked exception in `new EvalException(e)`, which takes over that message. So Starlark
/// sees `java.io.IOException: <message>` (or the subclass' fully qualified name) for an
/// `IOException` cause, and the bare message for an `EvalException` cause.
///
/// Every `RepositoryFunctionException` thrown by `StarlarkBaseExternalContext`,
/// `StarlarkRepositoryContext` and `StarlarkPath` goes through this function; the conversion
/// into [`JavaThrowable`] decides the class name.
// Java: com/google/devtools/build/lib/bazel/repository/RepositoryFunctionException.java:34 RepositoryFunctionException.<init>
// Java: com/google/devtools/build/skyframe/SkyFunctionException.java:64 SkyFunctionException.<init>
// Java: net/starlark/java/eval/MethodDescriptor.java:295 MethodDescriptor.call
pub fn repository_function_exception(cause: impl Into<JavaThrowable>) -> starlark::Error {
    eval_error(cause.into().to_java_string())
}

/// Throws the error of [`StarlarkBaseExternalContext::check_in_output_directory`]: `new
/// RepositoryFunctionException(Starlark.errorf(...), Transience.PERSISTENT)`.
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:334 StarlarkBaseExternalContext.checkInOutputDirectory
pub fn check_in_output_directory_error(message: String) -> starlark::Error {
    repository_function_exception(JavaThrowable::eval_exception(message))
}

/// What [`StarlarkBaseExternalContext::close`] throws.
#[derive(Debug, Clone)]
pub enum CloseError {
    /// Java `EvalException` ("Pending asynchronous work after ...").
    Eval(String),
    /// Java `IOException` from deleting the working directory.
    Io(IOException),
}

/// Whether to watch a path. See `read` for semantics.
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1704 StarlarkBaseExternalContext.ShouldWatch
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShouldWatch {
    Yes,
    No,
    Auto,
}

impl ShouldWatch {
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1709 ShouldWatch.fromString
    pub fn from_string(s: &str) -> starlark::Result<ShouldWatch> {
        match s {
            "yes" => Ok(ShouldWatch::Yes),
            "no" => Ok(ShouldWatch::No),
            "auto" => Ok(ShouldWatch::Auto),
            _ => Err(eval_error(format!(
                "bad value for 'watch' parameter; want 'yes', 'no', or 'auto', got {s}"
            ))),
        }
    }
}

/// The process-wrapper settings (Java `runtime.ProcessWrapper`).
// PORT: move to blaze_runtime (lib/runtime/ProcessWrapper.java) once it exists.
// Java: com/google/devtools/build/lib/runtime/ProcessWrapper.java:35 ProcessWrapper
struct ProcessWrapper {
    bin_path: PathFragment,
    kill_delay: Option<Duration>,
    graceful_sigterm: bool,
}

impl ProcessWrapper {
    // Java: com/google/devtools/build/lib/runtime/ProcessWrapper.java:92 ProcessWrapper.commandLineBuilder
    fn command_line_builder(&self, command_arguments: Vec<String>) -> ProcessWrapperCommandLine {
        ProcessWrapperCommandLine {
            process_wrapper_path: self.bin_path.clone(),
            command_arguments,
            kill_delay: self.kill_delay,
            graceful_sigterm: self.graceful_sigterm,
            timeout_millis: None,
        }
    }
}

// Java: com/google/devtools/build/lib/runtime/ProcessWrapper.java:105 ProcessWrapper.CommandLineBuilder
struct ProcessWrapperCommandLine {
    process_wrapper_path: PathFragment,
    command_arguments: Vec<String>,
    kill_delay: Option<Duration>,
    graceful_sigterm: bool,
    /// Java `Duration timeout`, kept in milliseconds (Java durations may be negative).
    timeout_millis: Option<i64>,
}

impl ProcessWrapperCommandLine {
    // Java: com/google/devtools/build/lib/runtime/ProcessWrapper.java:155 CommandLineBuilder.setTimeout
    fn set_timeout_millis(mut self, timeout_millis: i64) -> Self {
        self.timeout_millis = Some(timeout_millis);
        self
    }

    // Java: com/google/devtools/build/lib/runtime/ProcessWrapper.java:177 CommandLineBuilder.build
    fn build(self) -> Vec<String> {
        let mut full_command_line = Vec::new();
        full_command_line.push(self.process_wrapper_path.get_path_string().to_owned());
        if let Some(timeout_millis) = self.timeout_millis {
            // Java Duration.toSeconds(): floor division.
            full_command_line.push(format!("--timeout={}", timeout_millis.div_euclid(1000)));
        }
        // Java: `if (killDelay != null)`: a command without LocalExecutionOptions passes no
        // kill delay at all.
        if let Some(kill_delay) = self.kill_delay {
            full_command_line.push(format!("--kill_delay={}", kill_delay.as_secs()));
        }
        // stdoutPath, stderrPath and statisticsPath are never set by repository rules.
        if self.graceful_sigterm {
            full_command_line.push("--graceful_sigterm".to_owned());
        }
        full_command_line.extend(self.command_arguments);
        full_command_line
    }
}

/// The command line `execute` runs when there is a process-wrapper: Java
/// `processWrapper.commandLineBuilder(args).setTimeout(Duration.ofMillis(timeoutMillis)).build()`.
/// Repository rules set no stdout, stderr or statistics path.
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:2079 StarlarkBaseExternalContext.execute
fn process_wrapper_command_line(
    process_wrapper: &ProcessWrapper,
    args: Vec<String>,
    timeout_millis: i64,
) -> Vec<String> {
    process_wrapper
        .command_line_builder(args)
        .set_timeout_millis(timeout_millis)
        .build()
}

/// The state of a `repository_ctx` / `module_ctx` (Java `StarlarkBaseExternalContext` fields).
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:114 StarlarkBaseExternalContext
pub struct StarlarkBaseExternalContext {
    pub working_directory: Path,
    pub directories: LoadingDirectories,
    pub env: SkyEnv,
    pub repo_env: Arc<IndexMap<String, String>>,
    pub nonstrict_repo_env: Arc<IndexMap<String, String>>,
    pub download_manager: Option<Arc<DownloadManager>>,
    pub timeout_scaling: f64,
    pub process_wrapper: Option<Path>,
    /// Java `ProcessWrapper.killDelay` (`LocalExecutionOptions.getLocalSigkillGraceSeconds()`,
    /// `--local_termination_grace_seconds`, when the command has `LocalExecutionOptions`, else
    /// null: no `--kill_delay` argument).
    pub process_wrapper_kill_delay: Option<Duration>,
    /// Java `ProcessWrapper.gracefulSigterm` (`--experimental_process_wrapper_graceful_sigterm`).
    pub process_wrapper_graceful_sigterm: bool,
    /// Java `remoteExecutor` (null unless remote execution is configured).
    pub remote_executor:
        Option<Arc<dyn crate::repository::remote_helpers::RepositoryRemoteExecutor>>,
    pub starlark_semantics: Arc<StarlarkSemantics>,
    pub identifying_string_for_logging: String,
    os_object: StarlarkOS,
    recorded_inputs: IndexMap<RepoRecordedInput, Option<String>>,
    inputs_changed_during_evaluation: HashSet<RepoRecordedInput>,
    async_tasks: Vec<Arc<PendingDownloadTask>>,
    allow_watching_paths_outside_workspace: bool,
    was_successful: bool,
}

impl StarlarkBaseExternalContext {
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:178 StarlarkBaseExternalContext.<init>
    pub fn new(
        working_directory: Path,
        directories: LoadingDirectories,
        env: SkyEnv,
        repo_env: Arc<IndexMap<String, String>>,
        nonstrict_repo_env: Arc<IndexMap<String, String>>,
        download_manager: Option<Arc<DownloadManager>>,
        timeout_scaling: f64,
        process_wrapper: Option<Path>,
        starlark_semantics: Arc<StarlarkSemantics>,
        identifying_string_for_logging: String,
        allow_watching_paths_outside_workspace: bool,
    ) -> Self {
        let os_object = StarlarkOS::new(repo_env.clone());
        StarlarkBaseExternalContext {
            working_directory,
            directories,
            env,
            repo_env,
            nonstrict_repo_env,
            download_manager,
            timeout_scaling,
            process_wrapper,
            // Java: the `ProcessWrapper` object carries these (ProcessWrapper
            // .fromCommandEnvironment reads them from LocalExecutionOptions). Here they are the
            // values of a command without LocalExecutionOptions (`killDelay == null`, no
            // graceful SIGTERM) until the caller copies `RepoServices
            // .process_wrapper_kill_delay` / `.process_wrapper_graceful_sigterm` into them
            // (`with_process_wrapper_options`).
            process_wrapper_kill_delay: None,
            process_wrapper_graceful_sigterm: false,
            // PORT: Java's constructor takes it; callers set it with `with_remote_executor`.
            remote_executor: None,
            starlark_semantics,
            identifying_string_for_logging,
            os_object,
            recorded_inputs: IndexMap::new(),
            inputs_changed_during_evaluation: HashSet::new(),
            async_tasks: Vec::new(),
            allow_watching_paths_outside_workspace,
            was_successful: false,
        }
    }

    /// Sets the `killDelay` and `gracefulSigterm` of Java's `ProcessWrapper` object
    /// (`RepoServices::process_wrapper_kill_delay`, `process_wrapper_graceful_sigterm`).
    // Java: com/google/devtools/build/lib/runtime/ProcessWrapper.java:72 ProcessWrapper.fromCommandEnvironment
    pub fn with_process_wrapper_options(
        mut self,
        kill_delay: Option<Duration>,
        graceful_sigterm: bool,
    ) -> Self {
        self.process_wrapper_kill_delay = kill_delay;
        self.process_wrapper_graceful_sigterm = graceful_sigterm;
        self
    }

    /// Sets Java's `remoteExecutor` constructor argument.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:178 StarlarkBaseExternalContext.<init>
    pub fn with_remote_executor(
        mut self,
        remote_executor: Option<
            Arc<dyn crate::repository::remote_helpers::RepositoryRemoteExecutor>,
        >,
    ) -> Self {
        self.remote_executor = remote_executor;
        self
    }

    /// Marks the evaluation using this context as otherwise successful. This is used to
    /// determine how to clean up resources in [`Self::close`].
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:223 StarlarkBaseExternalContext.markSuccessful
    pub fn mark_successful(&mut self) {
        self.was_successful = true;
    }

    pub fn was_successful(&self) -> bool {
        self.was_successful
    }

    /// `should_delete` is `ExternalContext::should_delete_working_directory_on_close(
    /// was_successful())`.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:228 StarlarkBaseExternalContext.close
    pub fn close(&mut self, should_delete: bool) -> Result<(), CloseError> {
        // Cancel all pending async tasks.
        let had_pending_items = self.cancel_pending_async_tasks();
        // Wait for all (cancelled) async tasks to complete before cleaning up the working
        // directory. This is necessary because downloads may still be in progress and could end
        // up writing to the working directory during deletion, which would cause an error.
        for task in &self.async_tasks {
            task.close();
        }

        if should_delete && let Err(e) = self.working_directory.delete_tree() {
            return Err(CloseError::Io(IOException::from_io(&e)));
        }
        if had_pending_items && self.was_successful {
            return Err(CloseError::Eval(format!(
                "Pending asynchronous work after {} finished execution",
                self.identifying_string_for_logging
            )));
        }
        Ok(())
    }

    /// Java `repoMappingRecorder`: records an attempt of `Label()` to resolve an apparent repo
    /// name (see #20721).
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:212 StarlarkBaseExternalContext.<init>
    pub fn record_repo_mapping(
        &mut self,
        from_repo: &RepositoryName,
        apparent_repo_name: &str,
        canonical: &RepositoryName,
    ) {
        self.record_input_with_value(
            RepoRecordedInput::RecordedRepoMapping {
                source_repo: from_repo.clone(),
                apparent_name: apparent_repo_name.to_owned(),
            },
            if canonical.is_visible() {
                Some(canonical.get_name().to_owned())
            } else {
                None
            },
        );
    }

    /// Records the given value for the given input and returns the recorded value, which is the
    /// value observed first if the input has already been recorded.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:259 StarlarkBaseExternalContext.recordInputWithValue
    pub fn record_input_with_value(
        &mut self,
        input: RepoRecordedInput,
        value: Option<String>,
    ) -> Option<String> {
        // Don't use putIfAbsent as null is a legitimate value.
        let Some(recorded_value) = self.recorded_inputs.get(&input).cloned() else {
            self.recorded_inputs.insert(input, value.clone());
            return value;
        };
        if recorded_value != value && self.inputs_changed_during_evaluation.insert(input.clone()) {
            let message = format!(
                "{} during the evaluation of {}, which will cause it to be re-evaluated the next \
                 time Bazel is run. Report this issue to its maintainers.",
                input.describe_change(recorded_value.as_deref(), value.as_deref()),
                self.identifying_string_for_logging
            );
            self.env.get().listener().handle(&Event::warn(message));
        }
        recorded_value
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:295 StarlarkBaseExternalContext.cancelPendingAsyncTasks
    fn cancel_pending_async_tasks(&self) -> bool {
        let mut had_pending_items = false;
        for task in &self.async_tasks {
            if !task.cancel() {
                had_pending_items = true;
                if self.was_successful {
                    self.env.get().listener().handle(&Event::error_at(
                        Some(task.location.clone()),
                        format!(
                            "Work pending after {} finished execution: {}",
                            self.identifying_string_for_logging,
                            task.get_description()
                        ),
                    ));
                }
            }
        }
        had_pending_items
    }

    // There is no unregister(). We don't have that many futures in each repository and it just
    // introduces the failure mode of erroneously unregistering async work that's not done.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:317 StarlarkBaseExternalContext.registerAsyncTask
    fn register_async_task(&mut self, task: Arc<PendingDownloadTask>) {
        self.async_tasks.push(task);
    }

    /// Returns all recorded inputs in the order they were recorded.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:325 StarlarkBaseExternalContext.getRecordedInputs
    pub fn get_recorded_inputs(&self) -> Vec<RepoRecordedInputWithValue> {
        self.recorded_inputs
            .iter()
            .map(|(input, value)| RepoRecordedInputWithValue {
                input: input.clone(),
                value: value.clone(),
            })
            .collect()
    }

    /// `Err` is the message of the `EvalException` (`Starlark.errorf`) that Java wraps in a
    /// PERSISTENT `RepositoryFunctionException`; callers throw it with
    /// [`check_in_output_directory_error`].
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:331 StarlarkBaseExternalContext.checkInOutputDirectory
    pub fn check_in_output_directory(
        &self,
        operation: &str,
        path: &StarlarkPath,
    ) -> Result<(), String> {
        if !path.get_path().starts_with(&self.working_directory) {
            return Err(format!(
                "Cannot {} outside of the repository directory for path {}",
                operation, path
            ));
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:503 StarlarkBaseExternalContext.warnAboutChecksumError
    fn warn_about_checksum_error(&self, urls: &[JavaUri], error_message: &str) {
        // Inform the user immediately, even though the file will still be downloaded.
        // This cannot be done by a regular error event, as all regular events are recorded
        // and only shown once the execution of the repository rule is finished.
        // So we have to provide the information as update on the progress
        let url = match urls.first() {
            None => "(unknown)".to_owned(),
            Some(u) => u.to_string(),
        };
        self.report_progress(&format!(
            "Will fail after download of {}. {}",
            url, error_message
        ));
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:512 StarlarkBaseExternalContext.validateChecksum
    fn validate_checksum(
        &self,
        sha256: &str,
        integrity: &str,
        urls: &[JavaUri],
    ) -> Result<Option<Checksum>, ChecksumValidationError> {
        if !sha256.is_empty() {
            if !integrity.is_empty() {
                return Err(ChecksumValidationError::Eval(
                    "Expected either 'sha256' or 'integrity', but not both".to_owned(),
                ));
            }
            return match Checksum::from_string(KeyType::SHA256, sha256) {
                Ok(c) => Ok(Some(c)),
                Err(e) => {
                    self.warn_about_checksum_error(urls, &e.message);
                    Err(ChecksumValidationError::Repository(
                        JavaThrowable::eval_exception(format!(
                            "Checksum error in {}: {}",
                            self.identifying_string_for_logging, e.message
                        )),
                    ))
                }
            };
        }

        if integrity.is_empty() {
            return Ok(None);
        }

        match Checksum::from_subresource_integrity(integrity) {
            Ok(c) => Ok(Some(c)),
            Err(e) => {
                self.warn_about_checksum_error(urls, &e.message);
                Err(ChecksumValidationError::Repository(
                    JavaThrowable::eval_exception(format!(
                        "Checksum error in {}: {}",
                        self.identifying_string_for_logging, e.message
                    )),
                ))
            }
        }
    }

    /// Starts a download on its own thread (Java `downloadManager.startDownload(executorService,
    /// ...)`).
    ///
    /// PORT: `DownloadManager::start_download` returns an opaque handle that cannot be polled;
    /// this does the same (one thread running `DownloadManager::download`) but keeps the
    /// `JoinHandle` so `close()` can tell whether the download is still running. Blaze cannot
    /// interrupt a running download (Java cancels the future and waits on the `Phaser`).
    fn start_download(
        &self,
        urls: Vec<JavaUri>,
        headers: Headers,
        auth_headers: HashMap<JavaUri, Headers>,
        checksum: Option<Checksum>,
        canonical_id: String,
        type_: Option<String>,
        output: Path,
        may_hardlink: bool,
    ) -> JoinHandle<Result<Path, IoException>> {
        let download_manager = self
            .download_manager
            .clone()
            .expect("StarlarkBaseExternalContext.downloadManager is null");
        let client_env: HashMap<String, String> = self
            .nonstrict_repo_env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let context = self.identifying_string_for_logging.clone();
        std::thread::Builder::new()
            .name(format!("downloads[{}]-", context))
            .spawn(move || {
                download_manager.download(
                    &urls,
                    &headers,
                    &auth_headers,
                    checksum.as_ref(),
                    &canonical_id,
                    type_.as_deref(),
                    &output,
                    &client_env,
                    &context,
                    may_hardlink,
                )
            })
            .expect("failed to start download thread")
    }

    /// Creates the parent directories of `path`.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1790 StarlarkBaseExternalContext.makeDirectories
    pub fn make_directories(path: &Path) -> io::Result<()> {
        if let Some(parent) = path.get_parent_directory() {
            parent.create_directory_and_parents()?;
        }
        Ok(())
    }

    /// Updates the progress status for the fetching of this repository or module extension.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1807 StarlarkBaseExternalContext.reportProgress
    pub fn report_progress(&self, status: &str) {
        self.env.get().listener().post(Arc::new(ReportProgress {
            resource_identifier: self.identifying_string_for_logging.clone(),
            progress: status.to_owned(),
        }));
    }

    /// The `repository_os` object of this context (Java `osObject`).
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1832 StarlarkBaseExternalContext.getOS
    pub fn get_os(&self) -> StarlarkOS {
        self.os_object.clone()
    }

    /// `Err` is the cause of Java's `RepositoryFunctionException` (the `IOException`,
    /// TRANSIENT); callers throw it with [`repository_function_exception`].
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1843 StarlarkBaseExternalContext.createDirectory
    fn create_directory(directory: &Path) -> Result<(), JavaThrowable> {
        let r = (|| -> io::Result<()> {
            if !directory.exists() {
                Self::make_directories(directory)?;
                directory.create_directory()?;
            }
            Ok(())
        })();
        // PORT: Java's InvalidPathException branch has no counterpart (paths are not converted
        // to java.nio paths).
        r.map_err(JavaThrowable::from)
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:2366 StarlarkBaseExternalContext.findCommandOnPath
    fn find_command_on_path(&self, program: &str) -> io::Result<Option<StarlarkPath>> {
        let Some(path_env_variable) = self.repo_env.get("PATH") else {
            return Ok(None);
        };
        // Java String.split drops trailing empty strings; empty entries are relative anyway.
        for p in path_env_variable.split(':') {
            let fragment = PathFragment::create(p);
            if fragment.is_absolute() {
                // We ignore relative path as they don't mean much here (relative to where? the
                // workspace root?).
                let path = Path::create(fragment, self.working_directory.get_file_system())
                    .get_child(java_trim(program));
                if path.exists() && path.is_file() && path.is_executable()? {
                    return Ok(Some(StarlarkPath::new(path)));
                }
            }
        }
        Ok(None)
    }
}

/// Java `String.trim()`: strips characters `<= ' '` from both ends.
fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

/// Java `String.isBlank()`: empty or only `Character.isWhitespace` code points.
fn java_is_blank(s: &str) -> bool {
    s.chars().all(|c| {
        matches!(
            c,
            '\t' | '\n' | '\u{000B}' | '\u{000C}' | '\r' | '\u{001C}'..='\u{001F}'
        ) || (c.is_whitespace()
            && !matches!(c, '\u{00A0}' | '\u{2007}' | '\u{202F}')
            && !matches!(c, '\u{0085}'))
    })
}

/// Errors from `validateChecksum`: Java throws an `EvalException` (propagates immediately) or a
/// `RepositoryFunctionException` (deferred until the download finishes; this is its cause, an
/// `EvalException`).
enum ChecksumValidationError {
    Eval(String),
    Repository(JavaThrowable),
}

/// The `FetchProgress` posted by `report_progress` (Java anonymous class).
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1810 StarlarkBaseExternalContext.reportProgress
#[derive(Debug)]
struct ReportProgress {
    resource_identifier: String,
    progress: String,
}

impl Postable for ReportProgress {
    fn as_fetch_progress(&self) -> Option<&dyn FetchProgress> {
        Some(self)
    }
}

impl FetchProgress for ReportProgress {
    fn get_resource_identifier(&self) -> String {
        self.resource_identifier.clone()
    }

    fn get_progress(&self) -> String {
        self.progress.clone()
    }

    fn is_finished(&self) -> bool {
        false
    }
}

/// A progress event that reports about archive extraction.
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1403 StarlarkBaseExternalContext.ExtractProgress
#[derive(Debug)]
pub struct ExtractProgress {
    repository_path: String,
    progress: String,
    is_finished: bool,
}

impl ExtractProgress {
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1408 ExtractProgress.<init>
    fn new(repository_path: String, progress: String) -> Self {
        ExtractProgress {
            repository_path,
            progress,
            is_finished: false,
        }
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1414 ExtractProgress.<init>
    fn finished(repository_path: String) -> Self {
        ExtractProgress {
            repository_path,
            progress: String::new(),
            is_finished: true,
        }
    }
}

impl Postable for ExtractProgress {
    fn as_fetch_progress(&self) -> Option<&dyn FetchProgress> {
        Some(self)
    }
}

impl FetchProgress for ExtractProgress {
    fn get_resource_identifier(&self) -> String {
        self.repository_path.clone()
    }

    fn get_progress(&self) -> String {
        self.progress.clone()
    }

    fn is_finished(&self) -> bool {
        self.is_finished
    }
}

// -------------------------------------------------------------------------------------------
// Downloads.

/// An asynchronous download (Java `AsyncTask` + the download `Future<Path>` and `Phaser`).
#[derive(Debug)]
struct PendingDownloadTask {
    description: String,
    location: blaze_events::Location,
    future: Mutex<DownloadFuture>,
}

#[derive(Debug)]
struct DownloadFuture {
    handle: Option<JoinHandle<Result<Path, IoException>>>,
    /// The download's result; the error is the `IOException` Java's future fails with.
    result: Option<Result<Path, JavaThrowable>>,
}

impl PendingDownloadTask {
    fn running(
        description: String,
        location: blaze_events::Location,
        handle: JoinHandle<Result<Path, IoException>>,
    ) -> Self {
        PendingDownloadTask {
            description,
            location,
            future: Mutex::new(DownloadFuture {
                handle: Some(handle),
                result: None,
            }),
        }
    }

    /// Java `Futures.immediateFailedFuture(e)`.
    fn failed(description: String, location: blaze_events::Location, e: JavaThrowable) -> Self {
        PendingDownloadTask {
            description,
            location,
            future: Mutex::new(DownloadFuture {
                handle: None,
                result: Some(Err(e)),
            }),
        }
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:615 PendingDownload.getDescription
    fn get_description(&self) -> &str {
        &self.description
    }

    /// Returns false if the task was still in progress.
    ///
    /// PORT: Java cancels (interrupts) the download; Blaze cannot interrupt the download thread,
    /// so the download runs to completion and `close` waits for it.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:625 PendingDownload.cancel
    fn cancel(&self) -> bool {
        match &self.future.lock().handle {
            Some(handle) => handle.is_finished(),
            None => true,
        }
    }

    /// Waits until the task is no longer running.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:630 PendingDownload.close
    fn close(&self) {
        let mut future = self.future.lock();
        if let Some(handle) = future.handle.take() {
            let result = match handle.join() {
                Ok(r) => r,
                Err(panic) => std::panic::resume_unwind(panic),
            };
            future.result = Some(result.map_err(JavaThrowable::from));
        }
    }

    /// Java `downloadManager.finalizeDownload(future)`: the download's result.
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/DownloadManager.java:162 DownloadManager.finalizeDownload
    fn finalize(&self) -> Result<Path, JavaThrowable> {
        self.close();
        self.future
            .lock()
            .result
            .clone()
            .expect("the download finished")
    }
}

/// The token returned by `download(..., block=False)`.
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:585 StarlarkBaseExternalContext.PendingDownload
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct PendingDownload {
    #[allocative(skip)]
    executable: bool,
    #[allocative(skip)]
    allow_fail: bool,
    #[allocative(skip)]
    output_path: StarlarkPath,
    #[allocative(skip)]
    checksum: Option<Checksum>,
    /// The cause of the deferred checksum `RepositoryFunctionException`.
    #[allocative(skip)]
    checksum_validation: Option<JavaThrowable>,
    #[allocative(skip)]
    task: Arc<PendingDownloadTask>,
}

starlark_simple_value!(PendingDownload);

impl fmt::Display for PendingDownload {
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:653 PendingDownload.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<pending download to '{}'>", self.output_path)
    }
}

#[starlark_value(type = "PendingDownload")]
impl<'v> StarlarkValue<'v> for PendingDownload {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("PendingDownload", pending_download_methods);
        Some(RES.methods())
    }

    // Java: not immutable, so not hashable; identity equality (starlark-rust's defaults).
}

#[starlark_module]
fn pending_download_methods(builder: &mut MethodsBuilder) {
    /// Blocks until the completion of the download and returns or throws as blocking
    /// `download()` call would.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:648 PendingDownload.await
    fn wait<'v>(this: &PendingDownload, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        let outcome = complete_download(this)?;
        Ok(alloc_download_outcome(heap, &outcome))
    }
}

/// The result of a download: Java's `struct(success=False)` or the struct built by
/// `calculateDownloadResult`.
enum DownloadOutcome {
    Failed,
    Success { checksum: Checksum, size: i64 },
}

/// What `download` returns.
enum DownloadReturn {
    Pending(PendingDownload),
    Done(DownloadOutcome),
}

fn alloc_struct<'v>(heap: Heap<'v>, fields: Vec<(&str, Value<'v>)>) -> Value<'v> {
    let provider = heap.alloc(StructProvider);
    heap.alloc(StarlarkInfo::new(
        provider,
        fields.into_iter().map(|(k, v)| (k.to_owned(), v)).collect(),
    ))
}

// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:559 StarlarkBaseExternalContext.calculateDownloadResult
fn alloc_download_outcome<'v>(heap: Heap<'v>, outcome: &DownloadOutcome) -> Value<'v> {
    match outcome {
        DownloadOutcome::Failed => alloc_struct(heap, vec![("success", Value::new_bool(false))]),
        DownloadOutcome::Success { checksum, size } => {
            let mut out = vec![
                ("success", Value::new_bool(true)),
                (
                    "integrity",
                    heap.alloc_str(&checksum.to_subresource_integrity())
                        .to_value(),
                ),
            ];
            // For compatibility with older Bazel versions that don't support non-SHA256
            // checksums.
            if checksum.get_key_type() == KeyType::SHA256 {
                out.push(("sha256", heap.alloc_str(&checksum.to_string()).to_value()));
            }
            out.push(("size_bytes", heap.alloc(*size)));
            alloc_struct(heap, out)
        }
    }
}

// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:543 StarlarkBaseExternalContext.calculateChecksum
fn calculate_checksum(
    original_checksum: Option<&Checksum>,
    path: &Path,
) -> Result<Checksum, IoException> {
    if let Some(c) = original_checksum {
        // The checksum is checked on download, so if we got here, the user provided checksum is
        // good
        return Ok(c.clone());
    }
    let hex = download_cache::get_checksum(KeyType::SHA256, path)?;
    match Checksum::from_string(KeyType::SHA256, &hex) {
        Ok(c) => Ok(c),
        Err(_) => panic!(
            "IllegalStateException: Unexpected invalid checksum from internal computation of \
             SHA-256 checksum on {}",
            path.get_path_string()
        ),
    }
}

/// `Err` is the cause of Java's `RepositoryFunctionException` (a new `IOException`, PERSISTENT).
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:559 StarlarkBaseExternalContext.calculateDownloadResult
fn calculate_download_result(
    checksum: Option<&Checksum>,
    downloaded_path: &Path,
) -> Result<DownloadOutcome, JavaThrowable> {
    let r = (|| -> Result<(Checksum, i64), IoException> {
        let final_checksum = calculate_checksum(checksum, downloaded_path)?;
        let size = downloaded_path.get_file_size()?;
        Ok((final_checksum, size))
    })();
    match r {
        Ok((checksum, size)) => Ok(DownloadOutcome::Success { checksum, size }),
        Err(_) => Err(JavaThrowable::io_exception(format!(
            "Couldn't hash downloaded file ({})",
            downloaded_path.get_path_string()
        ))),
    }
}

// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:658 StarlarkBaseExternalContext.completeDownload
fn complete_download(pending_download: &PendingDownload) -> starlark::Result<DownloadOutcome> {
    let downloaded = (|| -> Result<Path, JavaThrowable> {
        let downloaded_path = pending_download.task.finalize()?;
        if pending_download.executable {
            pending_download
                .output_path
                .get_path()
                .set_executable(true)?;
        }
        Ok(downloaded_path)
    })();
    // Java: finally { pendingDownload.close(); } (finalize already waited for the task).
    let downloaded_path = match downloaded {
        Ok(p) => p,
        Err(e) => {
            if pending_download.allow_fail {
                return Ok(DownloadOutcome::Failed);
            }
            // Java: RepositoryFunctionException(e, Transience.TRANSIENT)
            return Err(repository_function_exception(e));
        }
    };
    if let Some(checksum_validation) = &pending_download.checksum_validation {
        // Java: throw pendingDownload.checksumValidation
        return Err(repository_function_exception(checksum_validation.clone()));
    }
    calculate_download_result(pending_download.checksum.as_ref(), &downloaded_path)
        .map_err(repository_function_exception)
}

/// This method wraps the deleteTree method in a retry loop, to solve an issue when trying to
/// recursively clean up temporary directories during dependency downloads when they are stored
/// on filesystems where unlinking a file may not be immediately reflected in a list of its parent
/// directory. See https://github.com/bazelbuild/bazel/issues/23687 and
/// https://github.com/bazelbuild/bazel/issues/20013.
///
/// `Err` is the cause of Java's `RepositoryFunctionException` (a new `IOException`, TRANSIENT).
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1212 StarlarkBaseExternalContext.deleteTreeWithRetries
fn delete_tree_with_retries(download_directory: &Path) -> Result<(), JavaThrowable> {
    let start = Instant::now();
    let deadline = start + Duration::from_secs(5);
    let mut attempts: i32 = 1;
    loop {
        let r = (|| -> io::Result<()> {
            if download_directory.exists() {
                download_directory.delete_tree()?;
            }
            Ok(())
        })();
        match r {
            Ok(()) => {
                // PORT: Java logs the number of attempts at INFO level (not user-visible).
                break;
            }
            Err(e) => {
                if Instant::now() > deadline {
                    // Java: RepositoryFunctionException(new IOException(..., e), TRANSIENT)
                    return Err(JavaThrowable::io_exception(format!(
                        "Couldn't delete temporary directory ({}) after {} attempts: {}",
                        download_directory.get_path_string(),
                        attempts,
                        JavaThrowable::from(&e).get_message()
                    )));
                }
            }
        }
        attempts += 1;
    }
    Ok(())
}

// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1436 StarlarkBaseExternalContext.renamedStripPrefix
fn renamed_strip_prefix(
    method: &str,
    strip_prefix: &str,
    old_strip_prefix: &str,
) -> starlark::Result<String> {
    if old_strip_prefix.is_empty() {
        return Ok(strip_prefix.to_owned());
    }
    if strip_prefix.is_empty() {
        return Ok(old_strip_prefix.to_owned());
    }
    Err(eval_error(format!(
        "{}() got multiple values for parameter 'strip_prefix' (via compatibility alias \
         'stripPrefix')",
        method
    )))
}

// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1450 StarlarkBaseExternalContext.validateStripping
fn validate_stripping(
    method: &str,
    strip_prefix: &str,
    strip_components: i32,
) -> starlark::Result<()> {
    if strip_components < 0 {
        return Err(eval_error(format!(
            "{}() has an invalid argument for 'strip_components': {}. Must be non-negative.",
            method, strip_components
        )));
    }

    if !strip_prefix.is_empty() && strip_components > 0 {
        return Err(eval_error(format!(
            "{}() got multiple strip values. Only one of 'strip_prefix' or 'strip_components' \
             can be set",
            method
        )));
    }
    Ok(())
}

// Do not manually edit. To get a ready-to-copy-and-paste string of updated decompression
// formats, run the test docSupportedFormats.
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:900 StarlarkBaseExternalContext.SUPPORTED_DECOMPRESSION_FORMATS
pub const SUPPORTED_DECOMPRESSION_FORMATS: &str = "\"zip\", \"jar\", \"war\", \"aar\", \
\"nupkg\", \"whl\", \"tar\", \"tar.gz\", \"tgz\", \"gz\", \"tar.xz\", \"txz\", \"xz\", \
\"tar.zst\", \"tzst\", \"zst\", \"tar.bz2\", \"tbz\", \"bz2\", \"ar\", \"deb\" or \"7z\"";

// -------------------------------------------------------------------------------------------
// Argument conversion (Java interpreter type checks, Dict.cast, Sequence.cast, toInt).

/// Java `BuiltinFunction` type error for a parameter.
fn param_type_error(method: &str, param: &str, value: Value<'_>, want: &str) -> starlark::Error {
    eval_error(format!(
        "in call to {}(), parameter '{}' got value of type '{}', want '{}'",
        method,
        param,
        value.get_type(),
        want
    ))
}

/// Checks a `string | Label | path` parameter.
pub fn check_path_param(method: &str, param: &str, value: Value<'_>) -> starlark::Result<()> {
    if value.unpack_str().is_some()
        || value.downcast_ref::<LabelValue>().is_some()
        || value.downcast_ref::<StarlarkPath>().is_some()
    {
        Ok(())
    } else {
        Err(param_type_error(
            method,
            param,
            value,
            "string, Label, or path",
        ))
    }
}

/// A `dict` parameter (default `{}`): its entries.
fn dict_param<'v>(
    method: &str,
    param: &str,
    value: Option<Value<'v>>,
) -> starlark::Result<Vec<(Value<'v>, Value<'v>)>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    match DictRef::from_value(value) {
        Some(d) => Ok(d.iter().collect()),
        None => Err(param_type_error(method, param, value, "dict")),
    }
}

/// Java `Dict.cast(x, String.class, <V>.class, what)` element check (`x` already a dict).
fn dict_cast_check(
    entries: &[(Value<'_>, Value<'_>)],
    what: &str,
    value_type: &str,
    value_ok: impl Fn(Value<'_>) -> bool,
) -> starlark::Result<()> {
    for (k, v) in entries {
        if k.unpack_str().is_none() || !value_ok(*v) {
            return Err(eval_error(format!(
                "got dict<{}, {}> for '{}', want dict<string, {}>",
                k.get_type(),
                v.get_type(),
                what,
                value_type
            )));
        }
    }
    Ok(())
}

/// `Dict.cast(x, String.class, String.class, what)` into a map.
fn string_dict(
    entries: &[(Value<'_>, Value<'_>)],
    what: &str,
) -> starlark::Result<IndexMap<String, String>> {
    dict_cast_check(entries, what, "string", |v| v.unpack_str().is_some())?;
    Ok(entries
        .iter()
        .map(|(k, v)| {
            (
                k.unpack_str().unwrap().to_owned(),
                v.unpack_str().unwrap().to_owned(),
            )
        })
        .collect())
}

/// An `int` parameter converted with Java `Starlark.toInt(x, what)`.
fn int_param(
    method: &str,
    param: &str,
    value: Option<Value<'_>>,
    default: i32,
) -> starlark::Result<IntParam> {
    let Some(value) = value else {
        return Ok(IntParam::Int(default));
    };
    if let Some(i) = value.unpack_i32() {
        return Ok(IntParam::Int(i));
    }
    if value.get_type() == "int" {
        return Ok(IntParam::OutOfRange(value.to_str()));
    }
    Err(param_type_error(method, param, value, "int"))
}

/// An `int` parameter: Java's interpreter accepts any `StarlarkInt`; the body's `toInt` fails
/// for values outside the 32-bit range.
enum IntParam {
    Int(i32),
    OutOfRange(String),
}

impl IntParam {
    // Java: net/starlark/java/eval/StarlarkInt.java:338 StarlarkInt.toInt
    fn to_int(&self, what: &str) -> starlark::Result<i32> {
        match self {
            IntParam::Int(i) => Ok(*i),
            IntParam::OutOfRange(s) => Err(eval_error(format!(
                "got {} for {}, want value in signed 32-bit range",
                s, what
            ))),
        }
    }
}

/// A `long` parameter: Java's interpreter accepts any `StarlarkInt`; the body's `toLong` fails
/// for values outside the 64-bit range.
enum LongParam {
    Long(i64),
    OutOfRange(String),
}

impl LongParam {
    // Java: net/starlark/java/eval/StarlarkInt.java:343 StarlarkInt.toLong
    fn to_long(&self, what: &str) -> starlark::Result<i64> {
        match self {
            LongParam::Long(l) => Ok(*l),
            LongParam::OutOfRange(s) => Err(eval_error(format!(
                "got {} for {}, want value in the signed 64-bit range",
                s, what
            ))),
        }
    }
}

/// `FileSystemUtils.readContent(p.getPath())`: the file's bytes.
// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java FileSystemUtils.readContent
fn read_content(p: &StarlarkPath) -> Result<Vec<u8>, JavaThrowable> {
    let mut stream = p
        .get_path()
        .get_input_stream()
        .map_err(|e| JavaThrowable::from_stream_open(&e))?;
    let mut bytes = Vec::new();
    io::Read::read_to_end(&mut stream, &mut bytes)
        .map_err(|e| JavaThrowable::from_stream_io(&e))?;
    Ok(bytes)
}

/// Java `String.valueOf(o)` / string concatenation of a Starlark value.
fn java_to_string(v: Value<'_>) -> String {
    if let Some(s) = v.unpack_str() {
        return s.to_owned();
    }
    if let Some(b) = v.unpack_bool() {
        return b.to_string();
    }
    v.to_repr()
}

/// The `Authorization` value of a `basic` auth entry: `"Basic " +
/// Base64.getEncoder().encodeToString(credentials.getBytes(UTF_8))`. `credentials` is an
/// internal string, so non-ASCII bytes are encoded twice, as in Java.
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:372 StarlarkBaseExternalContext.getAuthHeaders
fn basic_auth_value(credentials: &str) -> String {
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(credentials.as_bytes())
    )
}

/// Java `s.replaceAll(regex, replacement)` for a `regex` without metacharacters: replaces every
/// occurrence of `literal` by `replacement` expanded as `Matcher.appendReplacement` does (`\x`
/// is `x`, `$0` is the match; other group references fail).
fn java_replace_all(s: &str, literal: &str, replacement: &str) -> String {
    let chars: Vec<char> = replacement.chars().collect();
    let mut expanded = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' {
            i += 1;
            if i == chars.len() {
                panic!("IllegalArgumentException: character to be escaped is missing");
            }
            expanded.push(chars[i]);
            i += 1;
        } else if c == '$' {
            i += 1;
            if i == chars.len() {
                panic!("IllegalArgumentException: Illegal group reference: group index is missing");
            }
            if chars[i] == '{' {
                panic!("IllegalArgumentException: named group reference in replacement");
            }
            match chars[i].to_digit(10) {
                None => panic!("IllegalArgumentException: Illegal group reference"),
                Some(0) => {
                    i += 1;
                    // Further digits extend the group number only while it stays <= groupCount (0).
                    while i < chars.len() && chars[i] == '0' {
                        i += 1;
                    }
                    expanded.push_str(literal);
                }
                Some(n) => panic!("IndexOutOfBoundsException: No group {}", n),
            }
        } else {
            expanded.push(c);
            i += 1;
        }
    }
    s.replace(literal, &expanded)
}

/// From an authentication dict extract a map of headers.
///
/// Given a dict as provided as "auth" argument, compute a map specifying for each URI provided
/// which additional headers should additionally be added to the request.
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:350 StarlarkBaseExternalContext.getAuthHeaders
fn get_auth_headers(
    auth: &[(Value<'_>, Value<'_>)],
) -> starlark::Result<HashMap<JavaUri, Headers>> {
    // PORT: Java builds an ImmutableMap (iteration order unobservable: the downloader looks
    // URIs up by key).
    let mut headers = HashMap::new();
    for (key, value) in auth {
        let key = key.unpack_str().unwrap();
        let url = JavaUri::create(key).map_err(|e| eval_error(&e.message))?;
        let auth_map = DictRef::from_value(*value).unwrap();
        if let Some(type_) = auth_map.get_str("type") {
            if type_.unpack_str() == Some("basic") {
                let (Some(login), Some(password)) =
                    (auth_map.get_str("login"), auth_map.get_str("password"))
                else {
                    return Err(eval_error(format!(
                        "Found request to do basic auth for {} without 'login' and 'password' \
                         being provided.",
                        key
                    )));
                };
                let credentials = format!("{}:{}", java_to_string(login), java_to_string(password));
                let mut h = Headers::new();
                h.insert(
                    "Authorization".to_owned(),
                    vec![basic_auth_value(&credentials)],
                );
                headers.insert(url, h);
            } else if type_.unpack_str() == Some("pattern") {
                let Some(pattern) = auth_map.get_str("pattern") else {
                    return Err(eval_error(format!(
                        "Found request to do pattern auth for {} without a pattern being provided",
                        key
                    )));
                };

                let mut result = java_cast_string(pattern).to_owned();

                for component in ["password", "login"] {
                    let demarcated_component = format!("<{}>", component);

                    if result.contains(&demarcated_component) {
                        if auth_map.get_str(component).is_none() {
                            return Err(eval_error(format!(
                                "Auth pattern contains {} but it was not provided in auth dict.",
                                demarcated_component
                            )));
                        }
                    } else {
                        // component isn't in the pattern, ignore it
                        continue;
                    }

                    let replacement = java_cast_string(auth_map.get_str(component).unwrap());
                    result = java_replace_all(&result, &demarcated_component, replacement);
                }

                let mut h = Headers::new();
                h.insert("Authorization".to_owned(), vec![result]);
                headers.insert(url, h);
            }
        }
    }
    Ok(headers)
}

/// Java `(String) x`.
fn java_cast_string(v: Value<'_>) -> &str {
    match v.unpack_str() {
        Some(s) => s,
        None => panic!(
            "ClassCastException: class of Starlark type '{}' cannot be cast to class \
             java.lang.String",
            v.get_type()
        ),
    }
}

// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:409 StarlarkBaseExternalContext.getAuthContents
fn get_auth_contents<'v>(entries: &[(Value<'v>, Value<'v>)], what: &str) -> starlark::Result<()> {
    dict_cast_check(entries, what, "dict", |v| DictRef::from_value(v).is_some())
}

// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:417 StarlarkBaseExternalContext.getHeaderContents
fn get_header_contents<'v>(
    entries: &[(Value<'v>, Value<'v>)],
    what: &str,
    heap: Heap<'v>,
) -> starlark::Result<Headers> {
    dict_cast_check(entries, what, "unknown", |_| true)?;
    let mut headers = Headers::new();
    for (key, value_unchecked) in entries {
        let header_value: Vec<String> =
            if matches!(value_unchecked.get_type(), "list" | "tuple" | "range") {
                // Java: Sequence.cast(valueUnchecked, String.class, "header values")
                let mut values = Vec::new();
                for (i, elem) in value_unchecked.iterate(heap)?.enumerate() {
                    match elem.unpack_str() {
                        Some(s) => values.push(s.to_owned()),
                        None => {
                            return Err(eval_error(format!(
                                "at index {} of header values, got element of type {}, want string",
                                i,
                                elem.get_type()
                            )));
                        }
                    }
                }
                values
            } else if let Some(s) = value_unchecked.unpack_str() {
                vec![s.to_owned()]
            } else {
                return Err(eval_error(format!(
                    "{} argument must be a dict whose keys are string and whose values are either \
                 string or sequence of string",
                    what
                )));
            };
        headers.insert(key.unpack_str().unwrap().to_owned(), header_value);
    }
    Ok(headers)
}

/// The `url` parameter (`string | Iterable`) as strings (Java `getUrls` up to `checkAllUrls`).
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:443 StarlarkBaseExternalContext.checkAllUrls
fn url_strings<'v>(method: &str, url: Value<'v>, heap: Heap<'v>) -> starlark::Result<UrlParam> {
    if let Some(s) = url.unpack_str() {
        return Ok(UrlParam::Strings(vec![s.to_owned()]));
    }
    // Java Iterable: list, tuple, range, dict, set (not depset).
    if url.get_type() == "depset" {
        return Err(param_type_error(method, "url", url, "string or Iterable"));
    }
    let Ok(items) = url.iterate(heap) else {
        return Err(param_type_error(method, "url", url, "string or Iterable"));
    };
    let mut result = Vec::new();
    for o in items {
        match o.unpack_str() {
            Some(s) => result.push(s.to_owned()),
            None => {
                return Ok(UrlParam::BadItem(format!(
                    "Expected a string or sequence of strings for 'url' argument, but got '{}' \
                     item in the sequence",
                    o.get_type()
                )));
            }
        }
    }
    Ok(UrlParam::Strings(result))
}

/// The `url` parameter after the interpreter's type check; `checkAllUrls` fails in the body.
enum UrlParam {
    Strings(Vec<String>),
    BadItem(String),
}

// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:459 StarlarkBaseExternalContext.getUrls
fn get_urls(
    url: &UrlParam,
    ensure_non_empty: bool,
    checksum_given: bool,
) -> starlark::Result<Vec<JavaUri>> {
    let url_strings = match url {
        UrlParam::Strings(s) => s,
        UrlParam::BadItem(msg) => return Err(eval_error(msg)),
    };
    if ensure_non_empty && url_strings.is_empty() {
        // Java: RepositoryFunctionException(new IOException("urls not set"), PERSISTENT)
        return Err(repository_function_exception(JavaThrowable::io_exception(
            "urls not set",
        )));
    }
    let mut urls = Vec::new();
    for url_string in url_strings {
        let url = match JavaUri::create(url_string) {
            Ok(u) => u,
            Err(_) => {
                // Java: RepositoryFunctionException(new IOException("Bad URL: " + urlString, e),
                // PERSISTENT)
                return Err(repository_function_exception(JavaThrowable::io_exception(
                    format!("Bad URL: {}", url_string),
                )));
            }
        };
        if !http_utils::is_url_supported_by_downloader(&url) {
            // Java: RepositoryFunctionException(new IOException("Unsupported protocol: " +
            // url.getScheme()), PERSISTENT)
            return Err(repository_function_exception(JavaThrowable::io_exception(
                format!(
                    "Unsupported protocol: {}",
                    url.get_scheme().unwrap_or("null")
                ),
            )));
        }
        if !checksum_given {
            if !url
                .get_scheme()
                .is_some_and(|s| s.eq_ignore_ascii_case("http"))
            {
                urls.push(url);
            }
        } else {
            urls.push(url);
        }
    }
    if ensure_non_empty && urls.is_empty() {
        // Java: RepositoryFunctionException(new IOException(...), PERSISTENT)
        return Err(repository_function_exception(JavaThrowable::io_exception(
            "No URLs left after removing plain http URLs due to missing checksum. Please provide \
             either a checksum or an https download location.",
        )));
    }
    Ok(urls)
}

/// Java `thread.getCallerLocation()` as an event location.
fn caller_location(eval: &Evaluator<'_, '_, '_>) -> blaze_events::Location {
    match blaze_starlark::env::caller_location(eval) {
        Some(l) => blaze_events::Location::new(l.file, l.line as i32, l.column as i32),
        None => blaze_events::Location::builtin().clone(),
    }
}

/// An `execute` argument, classified by type.
enum ExecArg {
    Str(String),
    Label(Label),
    Path(StarlarkPath),
    Other,
}

// -------------------------------------------------------------------------------------------
// The methods that need the whole ExternalContext.

impl ExternalContext {
    /// `Ok(Err(message))` is Java's `IOException` (an invalid recorded input value).
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:283 StarlarkBaseExternalContext.getValueAndRecordInput
    fn get_value_and_record_input_or_io(
        &mut self,
        input: RepoRecordedInput,
    ) -> starlark::Result<Result<Option<String>, String>> {
        let maybe_value = match input.get_value(self.base.env.get(), &self.base.directories) {
            Ok(v) => v,
            Err(i) => return Err(self.interrupted(i)),
        };
        if self.base.env.get().values_missing() {
            return Err(self.needs_restart());
        }
        Ok(match maybe_value {
            MaybeValue::Invalid(reason) => Err(reason),
            MaybeValue::Valid(value) => Ok(self.base.record_input_with_value(input, value)),
        })
    }

    /// Java `getValueAndRecordInput` in a `try` whose `catch (IOException e)` throws `new
    /// RepositoryFunctionException(e, Transience.TRANSIENT)` (`maybeWatch`, `maybeWatchDirents`,
    /// `StarlarkRepositoryContext.watchTree`).
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:283 StarlarkBaseExternalContext.getValueAndRecordInput
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryContext.java:606 StarlarkRepositoryContext.watchTree
    pub fn get_value_and_record_input(
        &mut self,
        input: RepoRecordedInput,
    ) -> starlark::Result<Option<String>> {
        self.get_value_and_record_input_or_io(input)?
            .map_err(|reason| repository_function_exception(JavaThrowable::io_exception(reason)))
    }

    /// A path from a string (relative to the working directory), label, or path.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1593 StarlarkBaseExternalContext.getPath
    pub fn get_path(&mut self, path: Value<'_>) -> starlark::Result<StarlarkPath> {
        if let Some(s) = path.unpack_str() {
            return Ok(StarlarkPath::new(
                self.base.working_directory.get_relative_str(s),
            ));
        }
        if let Some(label) = path.downcast_ref::<LabelValue>() {
            let label = label.0.clone();
            return self.get_path_from_label(&label);
        }
        if let Some(starlark_path) = path.downcast_ref::<StarlarkPath>() {
            return Ok(starlark_path.clone());
        }
        // This can never happen because we check it in the Starlark interpreter.
        // PORT: Java throws IllegalArgumentException; callers that skip the parameter type
        // check get an error instead of a crash.
        Err(eval_error("expected string or label for path"))
    }

    /// Resolves the label into a file path.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:2386 StarlarkBaseExternalContext.getPathFromLabel
    pub fn get_path_from_label(&mut self, label: &Label) -> starlark::Result<StarlarkPath> {
        let rooted_path = match self.get_rooted_path_from_label(label)? {
            Some(p) => p,
            None => return Err(self.needs_restart()),
        };
        if !label.get_repository().is_main()
            && let Some(remote_fs) = crate::repository::remote_helpers::remote_repo_materializer()
            && let Err(e) = remote_fs
                .ensure_materialized(label.get_repository(), self.base.env.get().listener())
        {
            return Err(eval_error(&format!(
                "Failed to materialize remote repo {}: {}",
                label.get_repository(),
                e
            )));
        }
        let starlark_path = StarlarkPath::new(rooted_path.as_path());
        let should_watch = if self
            .base
            .starlark_semantics
            .incompatible_no_implicit_watch_label
        {
            ShouldWatch::No
        } else {
            ShouldWatch::Auto
        };
        // Java: catch (RepositoryFunctionException e) { throw Starlark.errorf("%s",
        // e.getCause().getMessage()); }: here the IOException's message, without its class.
        self.maybe_watch_or_io(&starlark_path, should_watch)?
            .map_err(eval_error)?;
        Ok(starlark_path)
    }

    // Java: com/google/devtools/build/lib/bazel/repository/RepositoryUtils.java:54 RepositoryUtils.getRootedPathFromLabel
    fn get_rooted_path_from_label(
        &mut self,
        label: &Label,
    ) -> starlark::Result<Option<RootedPath>> {
        match repository_utils::get_rooted_path_from_label(label, self.base.env.get()) {
            Ok(Ok(rooted_path)) => Ok(rooted_path),
            Ok(Err(message)) => Err(eval_error(message)),
            Err(i) => Err(self.interrupted(i)),
        }
    }

    /// Converts a path to a [`RepoCacheFriendlyPath`] based on `should_watch`. Returns `None`
    /// if the path shouldn't be watched; fails if watching is illegal but was requested.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1656 StarlarkBaseExternalContext.toRepoCacheFriendlyPath
    pub fn to_repo_cache_friendly_path(
        &mut self,
        path: &Path,
        should_watch: ShouldWatch,
    ) -> starlark::Result<Option<RepoCacheFriendlyPath>> {
        if should_watch == ShouldWatch::No {
            return Ok(None);
        }
        if path.starts_with(&self.base.working_directory) {
            // The path is under the working directory. Don't watch it, as it would cause a
            // dependency cycle.
            if should_watch == ShouldWatch::Auto {
                return Ok(None);
            }
            return Err(eval_error(
                "attempted to watch path under working directory",
            ));
        }
        let workspace = self.base.directories.get_workspace();
        if path.starts_with(workspace) {
            // The file is under the workspace root.
            let rel_path = path.relative_to(workspace);
            return Ok(Some(RepoCacheFriendlyPath::create_inside_workspace(
                MAIN.clone(),
                rel_path,
            )));
        }
        let output_base_external = self.base.directories.get_external_repository_directory();
        if path.starts_with(&output_base_external) {
            let rel_path = path.relative_to(&output_base_external);
            if !rel_path.is_empty() {
                // The file is under a repo root.
                let repo_name = match RepositoryName::create(rel_path.get_segment(0)) {
                    Ok(r) => r,
                    Err(e) => {
                        return Err(eval_error(format!(
                            "attempted to watch path under external repository directory: {}",
                            e.get_message()
                        )));
                    }
                };
                let repo_rel_path = rel_path.relative_to(&PathFragment::create_already_normalized(
                    repo_name.get_name(),
                ));
                return Ok(Some(RepoCacheFriendlyPath::create_inside_workspace(
                    repo_name,
                    repo_rel_path,
                )));
            }
        }
        // The file is just under a random absolute path.
        if !self.base.allow_watching_paths_outside_workspace {
            if should_watch == ShouldWatch::Auto {
                return Ok(None);
            }
            return Err(eval_error(
                "attempted to watch path outside workspace, but it's prohibited in the current \
                 context",
            ));
        }
        Ok(Some(RepoCacheFriendlyPath::create_outside_workspace(
            path.as_fragment().clone(),
        )))
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1721 StarlarkBaseExternalContext.maybeWatch
    pub fn maybe_watch(
        &mut self,
        starlark_path: &StarlarkPath,
        should_watch: ShouldWatch,
    ) -> starlark::Result<()> {
        // Java: catch (IOException e) { throw new RepositoryFunctionException(e, TRANSIENT); }
        self.maybe_watch_or_io(starlark_path, should_watch)?
            .map_err(|reason| repository_function_exception(JavaThrowable::io_exception(reason)))
    }

    /// The body of Java's `maybeWatch`: `Ok(Err(message))` is the `IOException` of
    /// `getValueAndRecordInput` (the cause of the `RepositoryFunctionException` `maybeWatch`
    /// throws, which `getPathFromLabel` unwraps again).
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1721 StarlarkBaseExternalContext.maybeWatch
    fn maybe_watch_or_io(
        &mut self,
        starlark_path: &StarlarkPath,
        should_watch: ShouldWatch,
    ) -> starlark::Result<Result<(), String>> {
        let Some(repo_cache_friendly_path) =
            self.to_repo_cache_friendly_path(starlark_path.get_path(), should_watch)?
        else {
            return Ok(Ok(()));
        };
        Ok(self
            .get_value_and_record_input_or_io(RepoRecordedInput::File(repo_cache_friendly_path))?
            .map(|_| ()))
    }

    /// Records a watch on a directory's non-recursive contents.
    ///
    /// Callers must have checked recently that the given path points to a directory.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1740 StarlarkBaseExternalContext.maybeWatchDirents
    pub fn maybe_watch_dirents(
        &mut self,
        path: &Path,
        should_watch: ShouldWatch,
    ) -> starlark::Result<()> {
        let Some(repo_cache_friendly_path) =
            self.to_repo_cache_friendly_path(path, should_watch)?
        else {
            return Ok(());
        };
        // Dirents can only be recorded for directories, so we have to additionally track the
        // type of the file. When checking for invalidation, the type is verified first and if it
        // doesn't match, the directory entries are never requested.
        // Java: catch (IOException e) { throw new RepositoryFunctionException(e, TRANSIENT); }
        self.get_value_and_record_input(RepoRecordedInput::File(repo_cache_friendly_path.clone()))?;
        self.get_value_and_record_input(RepoRecordedInput::Dirents(repo_cache_friendly_path))?;
        Ok(())
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:799 StarlarkBaseExternalContext.download
    fn download<'v>(
        &mut self,
        url: &UrlParam,
        output: Value<'v>,
        sha256: &str,
        executable: bool,
        allow_fail: bool,
        canonical_id: &str,
        auth: &[(Value<'v>, Value<'v>)],
        headers: &[(Value<'v>, Value<'v>)],
        integrity: &str,
        block: bool,
        location: blaze_events::Location,
        heap: Heap<'v>,
    ) -> starlark::Result<DownloadReturn> {
        get_auth_contents(auth, "auth")?;
        let auth_headers = get_auth_headers(auth)?;

        let headers = get_header_contents(headers, "headers", heap)?;

        let urls = get_urls(
            url,
            /* ensure_non_empty= */ !allow_fail,
            /* checksum_given= */ !sha256.is_empty() || !integrity.is_empty(),
        )?;
        let (checksum, checksum_validation) =
            match self.base.validate_checksum(sha256, integrity, &urls) {
                Ok(c) => (c, None),
                Err(ChecksumValidationError::Repository(e)) => (None, Some(e)),
                Err(ChecksumValidationError::Eval(e)) => return Err(eval_error(e)),
            };

        let output_path = self.get_path(output)?;
        // PORT: WorkspaceRuleEvent.newDownloadEvent not posted (see module docs).

        self.base
            .check_in_output_directory("write", &output_path)
            .map_err(check_in_output_directory_error)?;
        let description = format!("downloading to '{}'", output_path);
        let task = match StarlarkBaseExternalContext::make_directories(output_path.get_path()) {
            Err(e) => Arc::new(PendingDownloadTask::failed(
                description,
                location,
                JavaThrowable::from(e),
            )),
            Ok(()) => {
                let handle = self.base.start_download(
                    urls,
                    headers,
                    auth_headers,
                    checksum.clone(),
                    canonical_id.to_owned(),
                    None,
                    output_path.get_path().clone(),
                    // The repo rule may modify the file after the download, so we cannot
                    // guarantee that hardlinking is safe.
                    /* may_hardlink= */
                    false,
                );
                let task = Arc::new(PendingDownloadTask::running(description, location, handle));
                self.base.register_async_task(task.clone());
                task
            }
        };
        let download = PendingDownload {
            executable,
            allow_fail,
            output_path,
            checksum,
            checksum_validation,
            task,
        };
        if !block {
            Ok(DownloadReturn::Pending(download))
        } else {
            Ok(DownloadReturn::Done(complete_download(&download)?))
        }
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1064 StarlarkBaseExternalContext.downloadAndExtract
    fn download_and_extract<'v>(
        &mut self,
        url: &UrlParam,
        output: Value<'v>,
        sha256: &str,
        type_: &str,
        strip_prefix: &str,
        allow_fail: bool,
        canonical_id: &str,
        auth: &[(Value<'v>, Value<'v>)],
        headers: &[(Value<'v>, Value<'v>)],
        integrity: &str,
        rename_files: &[(Value<'v>, Value<'v>)],
        old_strip_prefix: &str,
        strip_components_i: &IntParam,
        location: blaze_events::Location,
        heap: Heap<'v>,
    ) -> starlark::Result<DownloadOutcome> {
        let strip_prefix =
            renamed_strip_prefix("download_and_extract", strip_prefix, old_strip_prefix)?;
        let strip_components = strip_components_i.to_int("strip_components")?;
        validate_stripping("download_and_extract", &strip_prefix, strip_components)?;
        get_auth_contents(auth, "auth")?;
        let auth_headers = get_auth_headers(auth)?;

        let headers = get_header_contents(headers, "headers", heap)?;

        let urls = get_urls(
            url,
            /* ensure_non_empty= */ !allow_fail,
            /* checksum_given= */ !sha256.is_empty() || !integrity.is_empty(),
        )?;
        let (checksum, checksum_validation) =
            match self.base.validate_checksum(sha256, integrity, &urls) {
                Ok(c) => (c, None),
                Err(ChecksumValidationError::Repository(e)) => (None, Some(e)),
                Err(ChecksumValidationError::Eval(e)) => return Err(eval_error(e)),
            };

        let rename_files_map = string_dict(rename_files, "rename_files")?;

        // PORT: WorkspaceRuleEvent.newDownloadAndExtractEvent not posted (see module docs).

        let output_path = self.get_path(output)?;
        self.base
            .check_in_output_directory("write", &output_path)
            .map_err(check_in_output_directory_error)?;
        StarlarkBaseExternalContext::create_directory(output_path.get_path())
            .map_err(repository_function_exception)?;

        let downloaded = (|| -> Result<(Path, Path), JavaThrowable> {
            // Download to temp directory inside the outputDirectory and delete it after
            // extraction
            let download_directory = output_path.get_path().create_temp_directory("temp")?;

            let pending_download = self.base.start_download(
                urls,
                headers,
                auth_headers,
                checksum.clone(),
                canonical_id.to_owned(),
                Some(type_.to_owned()),
                download_directory.clone(),
                // The archive is not going to be modified and not accessible to the user, so its
                // safe to hardlink.
                /* may_hardlink= */
                true,
            );
            // Ensure that the download is cancelled if the repo rule is restarted as it runs in
            // its own executor.
            let pending_task = Arc::new(PendingDownloadTask::running(
                format!("downloading to '{}'", output_path),
                location,
                pending_download,
            ));
            self.base.register_async_task(pending_task.clone());
            let downloaded_path = pending_task.finalize()?;
            Ok((downloaded_path, download_directory))
        })();
        let (downloaded_path, download_directory) = match downloaded {
            Ok(r) => r,
            Err(e) => {
                if allow_fail {
                    return Ok(DownloadOutcome::Failed);
                }
                // Java: RepositoryFunctionException(e, Transience.TRANSIENT)
                return Err(repository_function_exception(e));
            }
        };
        if let Some(checksum_validation) = checksum_validation {
            // Java: throw checksumValidation
            return Err(repository_function_exception(checksum_validation));
        }
        self.base
            .env
            .get()
            .listener()
            .post(Arc::new(ExtractProgress::new(
                output_path.get_path().to_string(),
                format!("Extracting {}", downloaded_path.get_base_name()),
            )));
        decompressor::decompress(
            &DecompressorDescriptor::builder()
                .set_context(&self.base.identifying_string_for_logging)
                .set_archive_path(downloaded_path.clone())
                .set_destination_path(output_path.get_path().clone())
                .set_prefix(&strip_prefix)
                .set_strip_components(strip_components)
                .set_rename_files(rename_files_map)
                .build(),
            // Type does NOT need to be passed here, as the existing code renames the archive
            // path to include the type extension. The decompression code then uses the file
            // extension to get the proper decompressor.
            /* force_decompressor_type= */
            None,
        )
        // Java: DecompressorValue.decompress throws RepositoryFunctionException.
        .map_err(repository_function_exception)?;
        self.base
            .env
            .get()
            .listener()
            .post(Arc::new(ExtractProgress::finished(
                output_path.get_path().to_string(),
            )));

        let download_result = calculate_download_result(checksum.as_ref(), &downloaded_path)
            .map_err(repository_function_exception)?;
        delete_tree_with_retries(&download_directory).map_err(repository_function_exception)?;
        Ok(download_result)
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1344 StarlarkBaseExternalContext.extract
    fn extract(
        &mut self,
        archive: Value<'_>,
        output: Value<'_>,
        strip_prefix: &str,
        rename_files: &[(Value<'_>, Value<'_>)],
        watch_archive: &str,
        old_strip_prefix: &str,
        strip_components_i: &IntParam,
        type_: &str,
    ) -> starlark::Result<()> {
        let strip_prefix = renamed_strip_prefix("extract", strip_prefix, old_strip_prefix)?;
        let strip_components = strip_components_i.to_int("strip_components")?;
        validate_stripping("extract", &strip_prefix, strip_components)?;
        let archive_path = self.get_path(archive)?;

        if !archive_path.exists() {
            // Java: RepositoryFunctionException(Starlark.errorf(...), Transience.TRANSIENT)
            return Err(repository_function_exception(
                JavaThrowable::eval_exception(format!(
                    "Archive path '{}' does not exist.",
                    archive_path
                )),
            ));
        }
        if archive_path.is_dir() {
            return Err(eval_error(format!(
                "attempting to extract a directory: {}",
                archive_path
            )));
        }
        self.maybe_watch(&archive_path, ShouldWatch::from_string(watch_archive)?)?;

        let output_path = self.get_path(output)?;
        self.base
            .check_in_output_directory("write", &output_path)
            .map_err(check_in_output_directory_error)?;

        let rename_files_map = string_dict(rename_files, "rename_files")?;

        // PORT: WorkspaceRuleEvent.newExtractEvent not posted (see module docs).

        self.base
            .env
            .get()
            .listener()
            .post(Arc::new(ExtractProgress::new(
                output_path.get_path().to_string(),
                format!("Extracting {}", archive_path.get_basename()),
            )));
        let force_type = if java_is_blank(type_) {
            None
        } else {
            Some(type_)
        };
        decompressor::decompress(
            &DecompressorDescriptor::builder()
                .set_context(&self.base.identifying_string_for_logging)
                .set_archive_path(archive_path.get_path().clone())
                .set_destination_path(output_path.get_path().clone())
                .set_prefix(&strip_prefix)
                .set_strip_components(strip_components)
                .set_rename_files(rename_files_map)
                .build(),
            force_type,
        )
        // Java: DecompressorValue.decompress throws RepositoryFunctionException.
        .map_err(repository_function_exception)?;
        self.base
            .env
            .get()
            .listener()
            .post(Arc::new(ExtractProgress::finished(
                output_path.get_path().to_string(),
            )));
        Ok(())
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1499 StarlarkBaseExternalContext.createFile
    fn create_file(
        &mut self,
        path: Value<'_>,
        content: &str,
        executable: bool,
        _legacy_utf8: bool,
    ) -> starlark::Result<()> {
        let p = self.get_path(path)?;
        // PORT: WorkspaceRuleEvent.newFileEvent not posted (see module docs).
        self.base
            .check_in_output_directory("write", &p)
            .map_err(check_in_output_directory_error)?;
        let r = (|| -> Result<(), JavaThrowable> {
            StarlarkBaseExternalContext::make_directories(p.get_path())?;
            p.get_path().delete()?;
            {
                let mut stream = p
                    .get_path()
                    .get_output_stream()
                    .map_err(|e| JavaThrowable::from_stream_open(&e))?;
                // Java: StringUnsafe.getInternalStringBytes(content).
                stream
                    .write_all(&latin1_encode(content))
                    .map_err(|e| JavaThrowable::from_stream_io(&e))?;
                stream
                    .flush()
                    .map_err(|e| JavaThrowable::from_stream_io(&e))?;
            }
            if executable {
                p.get_path().set_executable(true)?;
            }
            Ok(())
        })();
        // Java: RepositoryFunctionException(e, Transience.TRANSIENT). PORT: Java's
        // InvalidPathException branch has no counterpart.
        r.map_err(repository_function_exception)
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1557 StarlarkBaseExternalContext.getEnvironmentValue
    fn get_environment_value(&mut self, name: &str) -> starlark::Result<Option<String>> {
        match self.get_value_and_record_input_or_io(RepoRecordedInput::EnvVar(name.to_owned()))? {
            Ok(v) => Ok(v),
            Err(e) => panic!("IllegalStateException: getting EnvVar never throws IOException: {e}"),
        }
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1631 StarlarkBaseExternalContext.readFile
    fn read_file(&mut self, path: Value<'_>, watch: &str) -> starlark::Result<String> {
        let p = self.get_path(path)?;
        // PORT: WorkspaceRuleEvent.newReadEvent not posted (see module docs).
        self.maybe_watch(&p, ShouldWatch::from_string(watch)?)?;
        if p.is_dir() {
            return Err(eval_error(format!(
                "attempting to read() a directory: {}",
                p
            )));
        }
        // Java: FileSystemUtils.readContent(p.getPath(), ISO_8859_1)
        let r = (|| -> Result<Vec<u8>, JavaThrowable> {
            let mut stream = p
                .get_path()
                .get_input_stream()
                .map_err(|e| JavaThrowable::from_stream_open(&e))?;
            let mut bytes = Vec::new();
            io::Read::read_to_end(&mut stream, &mut bytes)
                .map_err(|e| JavaThrowable::from_stream_io(&e))?;
            Ok(bytes)
        })();
        match r {
            // Java: ISO-8859-1, one char per byte.
            Ok(bytes) => Ok(latin1_decode(&bytes).into_owned()),
            // Java: RepositoryFunctionException(e, Transience.TRANSIENT)
            Err(e) => Err(repository_function_exception(e)),
        }
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1861 StarlarkBaseExternalContext.canExecuteRemote
    fn can_execute_remote(&self) -> bool {
        let feature_enabled = self.base.starlark_semantics.experimental_repo_remote_exec;
        let remote_exec_enabled = self.base.remote_executor.is_some();
        feature_enabled && self.is_remotable() && remote_exec_enabled
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1870 StarlarkBaseExternalContext.getRemotePathFromLabel
    fn get_remote_path_from_label(
        &mut self,
        label: &Label,
    ) -> starlark::Result<(PathFragment, blaze_vfs::Path)> {
        let local_path = self.get_path_from_label(label)?.get_path().clone();
        let remote_path = label
            .get_package_identifier()
            .get_source_root()
            .get_relative_str(label.get_name());
        Ok((remote_path, local_path))
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1878 StarlarkBaseExternalContext.executeRemote
    fn execute_remote(
        &mut self,
        arguments_unchecked: Vec<ExecArg>, // <String> or <Label> expected
        timeout: i32,
        environment: &IndexMap<String, String>,
        quiet: bool,
        working_directory: &str,
    ) -> starlark::Result<StarlarkExecutionResult> {
        blaze_util::check_state!(self.can_execute_remote());

        // Java: ImmutableSortedMap.Builder.naturalOrder() (buildOrThrow: no duplicate keys).
        let mut inputs_builder: std::collections::BTreeMap<PathFragment, blaze_vfs::Path> =
            std::collections::BTreeMap::new();
        let mut arguments: Vec<String> = Vec::new();
        for argument_unchecked in arguments_unchecked {
            match argument_unchecked {
                ExecArg::Label(label) => {
                    let (remote_path, local_path) = self.get_remote_path_from_label(&label)?;
                    arguments.push(remote_path.to_string());
                    if let Some(existing) = inputs_builder.get(&remote_path) {
                        // Java: ImmutableSortedMap.Builder.buildOrThrow.
                        panic!(
                            "IllegalArgumentException: Multiple entries with same key: {}={} and {}={}",
                            remote_path, existing, remote_path, local_path
                        );
                    }
                    inputs_builder.insert(remote_path, local_path);
                }
                ExecArg::Str(s) => arguments.push(s),
                ExecArg::Path(_) | ExecArg::Other => unreachable!("validated as remotable"),
            }
        }

        // PORT: skipped, profiling (STARLARK_REPOSITORY_FN "remote").
        let execution_properties = self
            .get_remote_exec_properties()
            .map_err(|e| eval_error(e.to_string()))?;
        let remote_executor = self
            .base
            .remote_executor
            .clone()
            .expect("canExecuteRemote checked remoteExecutor");
        let result = remote_executor.execute(
            &arguments,
            &inputs_builder,
            &execution_properties,
            environment,
            Some(working_directory),
            // PORT: Java's Duration.ofSeconds takes a negative timeout as is; a Rust Duration
            // cannot be negative, so it is zero (no timeout in the Action) here.
            Duration::from_secs(timeout.max(0) as u64),
        );
        match result {
            Ok(result) => {
                // Java: new String(bytes, StandardCharsets.US_ASCII): a byte above 0x7F decodes
                // to U+FFFD.
                let ascii = |bytes: &[u8]| -> String {
                    bytes
                        .iter()
                        .map(|&b| if b < 0x80 { b as char } else { '\u{FFFD}' })
                        .collect()
                };
                let stdout = ascii(result.stdout());
                let stderr = ascii(result.stderr());

                if !quiet {
                    let out_err = blaze_events::io::OutErr::system_out_err();
                    out_err.print_out(&stdout);
                    out_err.print_err(&stderr);
                }

                Ok(StarlarkExecutionResult::new(
                    result.exit_code(),
                    stdout,
                    stderr,
                ))
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {
                Err(self.abort(EvaluationAbort::Interrupted))
            }
            Err(e) => Err(eval_error(format!("remote_execute failed: {}", e))),
        }
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1930 StarlarkBaseExternalContext.validateExecuteArguments
    fn validate_execute_arguments(&self, arguments: &[ExecArg]) -> starlark::Result<()> {
        let is_remotable = self.is_remotable();
        for (i, arg) in arguments.iter().enumerate() {
            if is_remotable {
                if !matches!(arg, ExecArg::Str(_) | ExecArg::Label(_)) {
                    return Err(eval_error(format!(
                        "Argument {} of execute is neither a label nor a string.",
                        i
                    )));
                }
            } else if !matches!(arg, ExecArg::Str(_) | ExecArg::Label(_) | ExecArg::Path(_)) {
                return Err(eval_error(format!(
                    "Argument {} of execute is neither a path, label, nor string.",
                    i
                )));
            }
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:2019 StarlarkBaseExternalContext.execute
    fn execute(
        &mut self,
        arguments: Vec<ExecArg>,
        timeout_i: &IntParam,
        unchecked_environment: &[(Value<'_>, Value<'_>)],
        quiet: bool,
        override_working_directory: &str,
    ) -> starlark::Result<StarlarkExecutionResult> {
        self.validate_execute_arguments(&arguments)?;
        let timeout = timeout_i.to_int("timeout")?;

        dict_cast_check(unchecked_environment, "environment", "unknown", |_| true)?;
        let mut force_repo_env_variables: IndexMap<String, String> = IndexMap::new();
        let mut remove_repo_env_variables: IndexSet<String> = IndexSet::new();
        for (key, value) in unchecked_environment {
            let key = key.unpack_str().unwrap().to_owned();
            if value.is_none() {
                remove_repo_env_variables.insert(key);
            } else if let Some(s) = value.unpack_str() {
                force_repo_env_variables.insert(key, s.to_owned());
            } else {
                return Err(eval_error(format!(
                    "environment values must be strings or None, got {}",
                    java_to_string(*value)
                )));
            }
        }

        if self.can_execute_remote() {
            // Remote execution only sees the explicitly set environment variables, so removing
            // env vars isn't necessary.
            return self.execute_remote(
                arguments,
                timeout,
                &force_repo_env_variables,
                quiet,
                override_working_directory,
            );
        }

        // Execute on the local/host machine

        let mut args: Vec<String> = Vec::with_capacity(arguments.len());
        for arg in arguments {
            match arg {
                ExecArg::Label(label) => args.push(self.get_path_from_label(&label)?.to_string()),
                // String or StarlarkPath expected
                ExecArg::Str(s) => args.push(s),
                ExecArg::Path(p) => args.push(p.to_string()),
                ExecArg::Other => unreachable!("validated above"),
            }
        }

        // PORT: WorkspaceRuleEvent.newExecuteEvent not posted (see module docs).
        StarlarkBaseExternalContext::create_directory(&self.base.working_directory)
            .map_err(repository_function_exception)?;

        // Java: Math.round(timeout * 1000L * timeoutScaling)
        let timeout_millis =
            ((timeout as i64 * 1000) as f64 * self.base.timeout_scaling + 0.5).floor() as i64;
        if let Some(process_wrapper) = &self.base.process_wrapper {
            let process_wrapper = ProcessWrapper {
                bin_path: process_wrapper.as_fragment().clone(),
                kill_delay: self.base.process_wrapper_kill_delay,
                graceful_sigterm: self.base.process_wrapper_graceful_sigterm,
            };
            args = process_wrapper_command_line(&process_wrapper, args, timeout_millis);
        }

        let working_directory_path = if !override_working_directory.is_empty() {
            self.base
                .working_directory
                .get_relative_str(override_working_directory)
        } else {
            self.base.working_directory.clone()
        };
        StarlarkBaseExternalContext::create_directory(&working_directory_path)
            .map_err(repository_function_exception)?;

        let result =
            StarlarkExecutionResult::builder(self.base.os_object.get_environment_variables())
                .add_arguments(args)
                .set_directory(JavaFile::new(working_directory_path.get_path_string()))
                .add_environment_variables(&force_repo_env_variables)
                .remove_environment_variables(&remove_repo_env_variables)
                .set_timeout(timeout_millis)
                .set_quiet(quiet)
                .execute();
        match result {
            Ok(r) => Ok(r),
            Err(_) => Err(self.abort(EvaluationAbort::Interrupted)),
        }
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:2336 StarlarkBaseExternalContext.which
    fn which(&mut self, program: &str) -> starlark::Result<Option<StarlarkPath>> {
        // PORT: WorkspaceRuleEvent.newWhichEvent not posted (see module docs).
        if program.contains('/') || program.contains('\\') {
            return Err(eval_error(format!(
                "Program argument of which() may not contain a / or a \\ ('{}' given)",
                program
            )));
        }
        if program.is_empty() {
            return Err(eval_error("Program argument of which() may not be empty"));
        }
        match self.base.find_command_on_path(program) {
            Ok(Some(command_path)) => Ok(Some(command_path)),
            // OsUtils.executableExtension() is empty on Linux, so `program` always ends with it
            // and Java's second lookup never runs.
            Ok(None) => Ok(None),
            // IOException when checking executable file means we cannot read the file data so we
            // cannot execute it, swallow the exception.
            Err(_) => Ok(None),
        }
    }
}

/// The `os` of a `repository_ctx` / `module_ctx` handle.
fn os_of_ctx(this: Value<'_>) -> starlark::Result<StarlarkOS> {
    if let Some(ctx) = this.downcast_ref::<RepositoryCtx>() {
        return Ok(ctx.os.clone());
    }
    if let Some(ctx) = this.downcast_ref::<ModuleCtx>() {
        return Ok(ctx.os.clone());
    }
    Err(eval_error(
        "internal error: 'os' on a value that is not a repository_ctx or module_ctx",
    ))
}

/// The value for a `''` default of a `string | Label | path` parameter.
fn empty_string(heap: Heap<'_>) -> Value<'_> {
    heap.alloc_str("").to_value()
}

/// Java's error for a method hidden by `enableOnlyWithFlag`.
fn no_such_method(this: Value<'_>, name: &str) -> starlark::Error {
    eval_error(format!(
        "'{}' value has no field or method '{}'",
        this.get_type(),
        name
    ))
}

/// The Starlark methods of `StarlarkBaseExternalContext`, registered on both `repository_ctx`
/// and `module_ctx`.
#[starlark_module]
pub fn base_external_context_methods(builder: &mut MethodsBuilder) {
    /// Downloads a file to the output path for the provided url.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:799 StarlarkBaseExternalContext.download
    fn download<'v>(
        #[starlark(this)] _this: Value<'v>,
        url: Value<'v>,
        output: Option<Value<'v>>,
        #[starlark(default = "")] sha256: &str,
        #[starlark(default = false)] executable: bool,
        #[starlark(default = false)] allow_fail: bool,
        #[starlark(default = "")] canonical_id: &str,
        auth: Option<Value<'v>>,
        headers: Option<Value<'v>>,
        #[starlark(require = named, default = "")] integrity: &str,
        #[starlark(require = named, default = true)] block: bool,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        let heap = eval.heap();
        let url = url_strings("download", url, heap)?;
        let output = output.unwrap_or_else(|| empty_string(heap));
        check_path_param("download", "output", output)?;
        let auth = dict_param("download", "auth", auth)?;
        let headers = dict_param("download", "headers", headers)?;
        let location = caller_location(eval);
        let result = {
            let mut ctx = ExternalContext::from_eval(eval)?;
            ctx.download(
                &url,
                output,
                sha256,
                executable,
                allow_fail,
                canonical_id,
                &auth,
                &headers,
                integrity,
                block,
                location,
                heap,
            )?
        };
        Ok(match result {
            DownloadReturn::Pending(p) => heap.alloc(p),
            DownloadReturn::Done(outcome) => alloc_download_outcome(heap, &outcome),
        })
    }

    /// Downloads a file to the output path for the provided url, extracts it, and returns a
    /// struct.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1064 StarlarkBaseExternalContext.downloadAndExtract
    fn download_and_extract<'v>(
        #[starlark(this)] _this: Value<'v>,
        url: Value<'v>,
        output: Option<Value<'v>>,
        #[starlark(default = "")] sha256: &str,
        #[starlark(default = "")] r#type: &str,
        #[starlark(default = "")] strip_prefix: &str,
        #[starlark(default = false)] allow_fail: bool,
        #[starlark(default = "")] canonical_id: &str,
        auth: Option<Value<'v>>,
        headers: Option<Value<'v>>,
        #[starlark(require = named, default = "")] integrity: &str,
        #[starlark(require = named)] rename_files: Option<Value<'v>>,
        #[starlark(require = named, default = "")] stripPrefix: &str,
        #[starlark(require = named)] strip_components: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        let heap = eval.heap();
        let url = url_strings("download_and_extract", url, heap)?;
        let output = output.unwrap_or_else(|| empty_string(heap));
        check_path_param("download_and_extract", "output", output)?;
        let auth = dict_param("download_and_extract", "auth", auth)?;
        let headers = dict_param("download_and_extract", "headers", headers)?;
        let rename_files = dict_param("download_and_extract", "rename_files", rename_files)?;
        let strip_components = int_param(
            "download_and_extract",
            "strip_components",
            strip_components,
            0,
        )?;
        let location = caller_location(eval);
        let outcome = {
            let mut ctx = ExternalContext::from_eval(eval)?;
            ctx.download_and_extract(
                &url,
                output,
                sha256,
                r#type,
                strip_prefix,
                allow_fail,
                canonical_id,
                &auth,
                &headers,
                integrity,
                &rename_files,
                stripPrefix,
                &strip_components,
                location,
                heap,
            )?
        };
        Ok(alloc_download_outcome(heap, &outcome))
    }

    /// Extract an archive to the repository directory.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1344 StarlarkBaseExternalContext.extract
    fn extract<'v>(
        #[starlark(this)] _this: Value<'v>,
        archive: Value<'v>,
        output: Option<Value<'v>>,
        #[starlark(default = "")] strip_prefix: &str,
        #[starlark(require = named)] rename_files: Option<Value<'v>>,
        #[starlark(require = named, default = "auto")] watch_archive: &str,
        #[starlark(require = named, default = "")] stripPrefix: &str,
        #[starlark(require = named)] strip_components: Option<Value<'v>>,
        #[starlark(require = named, default = "")] r#type: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneType> {
        let heap = eval.heap();
        check_path_param("extract", "archive", archive)?;
        let output = output.unwrap_or_else(|| empty_string(heap));
        check_path_param("extract", "output", output)?;
        let rename_files = dict_param("extract", "rename_files", rename_files)?;
        let strip_components = int_param("extract", "strip_components", strip_components, 0)?;
        let mut ctx = ExternalContext::from_eval(eval)?;
        ctx.extract(
            archive,
            output,
            strip_prefix,
            &rename_files,
            watch_archive,
            stripPrefix,
            &strip_components,
            r#type,
        )?;
        Ok(NoneType)
    }

    /// Generates a file in the repository directory with the provided content.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1499 StarlarkBaseExternalContext.createFile
    fn file<'v>(
        #[starlark(this)] _this: Value<'v>,
        #[starlark(require = pos)] path: Value<'v>,
        #[starlark(default = "")] content: &str,
        #[starlark(default = true)] executable: bool,
        #[starlark(default = false)] legacy_utf8: bool,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneType> {
        check_path_param("file", "path", path)?;
        let mut ctx = ExternalContext::from_eval(eval)?;
        ctx.create_file(path, content, executable, legacy_utf8)?;
        Ok(NoneType)
    }

    /// Returns the value of an environment variable `name` as a string if exists, or `default`
    /// if it doesn't.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1557 StarlarkBaseExternalContext.getEnvironmentValue
    fn getenv<'v>(
        #[starlark(this)] _this: Value<'v>,
        #[starlark(require = pos)] name: &str,
        #[starlark(require = pos)] default: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        let default = default.unwrap_or_else(Value::new_none);
        if !default.is_none() && default.unpack_str().is_none() {
            return Err(param_type_error(
                "getenv",
                "default",
                default,
                "string or NoneType",
            ));
        }
        let value = {
            let mut ctx = ExternalContext::from_eval(eval)?;
            ctx.get_environment_value(name)?
        };
        Ok(match value {
            Some(v) => eval.heap().alloc_str(&v).to_value(),
            // Java: nullIfNone(defaultValue, String.class)
            None => default,
        })
    }

    /// Returns a path from a string, label, or path.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1593 StarlarkBaseExternalContext.getPath
    fn path<'v>(
        #[starlark(this)] _this: Value<'v>,
        #[starlark(require = pos)] path: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<StarlarkPath> {
        check_path_param("path", "path", path)?;
        let mut ctx = ExternalContext::from_eval(eval)?;
        ctx.get_path(path)
    }

    /// Reads the content of a file on the filesystem.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1631 StarlarkBaseExternalContext.readFile
    fn read<'v>(
        #[starlark(this)] _this: Value<'v>,
        #[starlark(require = pos)] path: Value<'v>,
        #[starlark(require = named, default = "auto")] watch: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<String> {
        check_path_param("read", "path", path)?;
        let mut ctx = ExternalContext::from_eval(eval)?;
        ctx.read_file(path, watch)
    }

    /// Tells Bazel to watch for changes to the given path.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1784 StarlarkBaseExternalContext.watchForStarlark
    fn watch<'v>(
        #[starlark(this)] _this: Value<'v>,
        #[starlark(require = pos)] path: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneType> {
        check_path_param("watch", "path", path)?;
        let mut ctx = ExternalContext::from_eval(eval)?;
        let p = ctx.get_path(path)?;
        ctx.maybe_watch(&p, ShouldWatch::Yes)?;
        Ok(NoneType)
    }

    /// Updates the progress status for the fetching of this repository or module extension.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1807 StarlarkBaseExternalContext.reportProgress
    fn report_progress<'v>(
        #[starlark(this)] _this: Value<'v>,
        #[starlark(require = pos, default = "")] status: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneType> {
        let ctx = ExternalContext::from_eval(eval)?;
        ctx.base.report_progress(status);
        Ok(NoneType)
    }

    /// A struct to access information from the system.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1832 StarlarkBaseExternalContext.getOS
    #[starlark(attribute)]
    fn os<'v>(this: Value<'v>) -> starlark::Result<StarlarkOS> {
        // PORT: WorkspaceRuleEvent.newOsEvent not posted (see module docs).
        os_of_ctx(this)
    }

    /// Executes the command given by the list of arguments.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:2019 StarlarkBaseExternalContext.execute
    fn execute<'v>(
        #[starlark(this)] _this: Value<'v>,
        #[starlark(require = pos)] arguments: Value<'v>,
        timeout: Option<Value<'v>>,
        environment: Option<Value<'v>>,
        #[starlark(default = true)] quiet: bool,
        #[starlark(default = "")] working_directory: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<StarlarkExecutionResult> {
        let heap = eval.heap();
        // Java: parameter type Sequence (list, tuple, range).
        if !matches!(arguments.get_type(), "list" | "tuple" | "range") {
            return Err(param_type_error(
                "execute",
                "arguments",
                arguments,
                "sequence",
            ));
        }
        let mut args = Vec::new();
        for arg in arguments.iterate(heap)? {
            args.push(if let Some(s) = arg.unpack_str() {
                ExecArg::Str(s.to_owned())
            } else if let Some(l) = arg.downcast_ref::<LabelValue>() {
                ExecArg::Label(l.0.clone())
            } else if let Some(p) = arg.downcast_ref::<StarlarkPath>() {
                ExecArg::Path(p.clone())
            } else {
                ExecArg::Other
            });
        }
        let timeout = int_param("execute", "timeout", timeout, 600)?;
        let environment = dict_param("execute", "environment", environment)?;
        let mut ctx = ExternalContext::from_eval(eval)?;
        ctx.execute(args, &timeout, &environment, quiet, working_directory)
    }

    /// Load a WebAssembly module from a file on the filesystem.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:2166 StarlarkBaseExternalContext.loadWasm
    fn load_wasm<'v>(
        #[starlark(this)] this: Value<'v>,
        path: Value<'v>,
        #[starlark(require = named, default = "allocate")] allocate_fn: &str,
        #[starlark(require = named, default = "auto")] watch: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        // Java: enableOnlyWithFlag = EXPERIMENTAL_REPOSITORY_CTX_EXECUTE_WASM.
        // PORT: starlark-rust cannot hide a method; it fails when called instead.
        let enabled = {
            let ctx = ExternalContext::from_eval(eval)?;
            ctx.base
                .starlark_semantics
                .experimental_repository_ctx_execute_wasm
        };
        if !enabled {
            return Err(no_such_method(this, "load_wasm"));
        }
        check_path_param("load_wasm", "path", path)?;
        let heap = eval.heap();
        let mut ctx = ExternalContext::from_eval(eval)?;
        let p = ctx.get_path(path)?;
        // PORT: WorkspaceRuleEvent.newLoadWasmEvent not posted (see module docs).
        ctx.maybe_watch(&p, ShouldWatch::from_string(watch)?)?;
        let module_content = read_content(&p).map_err(repository_function_exception)?;
        super::wasm::new_wasm_module(heap, p, path, &module_content, allocate_fn)
    }

    /// Instantiate a WebAssembly module and execute the specified function.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:2271 StarlarkBaseExternalContext.executeWasm
    fn execute_wasm<'v>(
        #[starlark(this)] this: Value<'v>,
        module: Value<'v>,
        function: Value<'v>,
        #[starlark(require = named)] input: Value<'v>,
        #[starlark(require = named)] timeout: Option<Value<'v>>,
        #[starlark(require = named)] memory_limit: Option<Value<'v>>,
        #[starlark(require = named, default = "auto")] watch: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        // Java: enableOnlyWithFlag = EXPERIMENTAL_REPOSITORY_CTX_EXECUTE_WASM.
        let enabled = {
            let ctx = ExternalContext::from_eval(eval)?;
            ctx.base
                .starlark_semantics
                .experimental_repository_ctx_execute_wasm
        };
        if !enabled {
            return Err(no_such_method(this, "execute_wasm"));
        }
        // The interpreter checks every argument against its `@Param` before the method runs.
        let wasm_module = super::wasm::wasm_module_of(module);
        if wasm_module.is_none() {
            check_path_param("execute_wasm", "module", module).map_err(|_| {
                param_type_error(
                    "execute_wasm",
                    "module",
                    module,
                    "string, Label, path, or wasm_module",
                )
            })?;
        }
        let Some(function) = function.unpack_str() else {
            return Err(param_type_error(
                "execute_wasm",
                "function",
                function,
                "string",
            ));
        };
        let Some(input) = input.unpack_str() else {
            return Err(param_type_error("execute_wasm", "input", input, "string"));
        };
        let timeout_i = int_param("execute_wasm", "timeout", timeout, 600)?;
        let mem_limit_i = match memory_limit {
            None => LongParam::Long(67108864),
            Some(v) => match <i64 as starlark::values::UnpackValue>::unpack_value(v)
                .ok()
                .flatten()
            {
                Some(l) => LongParam::Long(l),
                None if v.get_type() == "int" => LongParam::OutOfRange(v.to_str()),
                None => {
                    return Err(param_type_error("execute_wasm", "memory_limit", v, "int"));
                }
            },
        };

        let heap = eval.heap();
        let mut ctx = ExternalContext::from_eval(eval)?;
        let path = match &wasm_module {
            Some(m) => m.get_path().clone(),
            None => ctx.get_path(module)?,
        };

        // StringUnsafe.getInternalStringBytes(input): the internal (Latin-1) bytes.
        let input_bytes = blaze_util::string_encoding::latin1_encode(input).into_owned();
        let timeout_seconds = timeout_i.to_int("timeout")?;
        let mem_limit = mem_limit_i.to_long("memory_limit")?;

        // PORT: WorkspaceRuleEvent.newExecuteWasmEvent not posted (see module docs).

        // Math.round(timeoutSeconds * 1000L * timeoutScaling)
        let timeout_millis = ((timeout_seconds as i64 * 1000) as f64 * ctx.base.timeout_scaling
            + 0.5)
            .floor() as i64;
        let timeout = std::time::Duration::from_millis(timeout_millis.max(0) as u64);

        let result = match wasm_module {
            Some(m) => m.execute(function, &input_bytes, timeout, mem_limit)?,
            None => {
                ctx.maybe_watch(&path, ShouldWatch::from_string(watch)?)?;
                let module_content = read_content(&path).map_err(repository_function_exception)?;
                let loaded =
                    super::wasm::new_wasm_module(heap, path, module, &module_content, "allocate")?;
                super::wasm::wasm_module_of(loaded)
                    .expect("a wasm_module")
                    .execute(function, &input_bytes, timeout, mem_limit)?
            }
        };
        Ok(heap.alloc(result))
    }

    /// Returns the `path` of the corresponding program or `None` if there is no such program in
    /// the path.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:2336 StarlarkBaseExternalContext.which
    fn which<'v>(
        #[starlark(this)] _this: Value<'v>,
        #[starlark(require = pos)] program: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneOr<StarlarkPath>> {
        let mut ctx = ExternalContext::from_eval(eval)?;
        Ok(match ctx.which(program)? {
            Some(p) => NoneOr::Other(p),
            None => NoneOr::None,
        })
    }
}

#[cfg(test)]
mod tests {
    //! Port of src/test/java/com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContextTest.java

    use super::*;

    #[test]
    fn doc_supported_formats() {
        let expected = decompressor::readable_supported_formats("\"", "\"", "or");
        assert_eq!(SUPPORTED_DECOMPRESSION_FORMATS, expected);
    }

    fn process_wrapper(
        bin_path: &str,
        kill_delay: Option<Duration>,
        graceful_sigterm: bool,
    ) -> ProcessWrapper {
        ProcessWrapper {
            bin_path: PathFragment::create(bin_path),
            kill_delay,
            graceful_sigterm,
        }
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/ProcessWrapperTest.java:35
    // ProcessWrapperTest.testProcessWrapperCommandLineBuilder_buildsWithoutOptionalArguments
    #[test]
    fn test_process_wrapper_command_line_builder_builds_without_optional_arguments() {
        let command_arguments = vec!["echo".to_owned(), "hello, world".to_owned()];

        let expected_command_line = vec!["/some/path", "echo", "hello, world"];

        let process_wrapper = process_wrapper(
            "/some/path",
            /* kill_delay= */ None,
            /* graceful_sigterm= */ false,
        );
        let command_line = process_wrapper
            .command_line_builder(command_arguments)
            .build();

        assert_eq!(command_line, expected_command_line);
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/ProcessWrapperTest.java:53
    // ProcessWrapperTest.testProcessWrapperCommandLineBuilder_buildsWithOptionalArguments
    // PORT: only the optional arguments this crate's builder has (timeout, kill delay, graceful
    // SIGTERM); the builder with the stdout, stderr and statistics paths and the path override
    // is `blaze_exec::runtime::process_wrapper`, which ports the whole Java test.
    #[test]
    fn test_process_wrapper_command_line_builder_builds_with_optional_arguments() {
        let command_arguments = vec!["echo".to_owned(), "hello, world".to_owned()];

        let timeout = Duration::from_secs(10);
        let kill_delay = Duration::from_secs(2);

        let expected_command_line = vec![
            "/path/process-wrapper".to_owned(),
            format!("--timeout={}", timeout.as_secs()),
            format!("--kill_delay={}", kill_delay.as_secs()),
            "--graceful_sigterm".to_owned(),
            "echo".to_owned(),
            "hello, world".to_owned(),
        ];

        let process_wrapper = process_wrapper(
            "/path/process-wrapper",
            Some(kill_delay),
            /* graceful_sigterm= */ true,
        );
        let command_line = process_wrapper
            .command_line_builder(command_arguments)
            .set_timeout_millis(timeout.as_millis() as i64)
            .build();

        assert_eq!(command_line, expected_command_line);
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/ProcessWrapperTest.java:96
    // ProcessWrapperTest.testProcessWrapperCommandLineBuilder_withExecutionInfo
    #[test]
    #[ignore = "this crate's process-wrapper builder has no addExecutionInfo (repository rules \
                have no execution info); the test is ported with the full builder in \
                blaze_exec/tests/local_process_wrapper_test.rs"]
    fn test_process_wrapper_command_line_builder_with_execution_info() {}

    /// The command line `execute` builds for the three states of Java's `ProcessWrapper`:
    /// options absent (`killDelay == null`), `--local_termination_grace_seconds=15` (the
    /// default), and that with `--experimental_process_wrapper_graceful_sigterm`. The order is
    /// `CommandLineBuilder.build`'s: path, `--timeout`, `--kill_delay`, `--graceful_sigterm`,
    /// then the command.
    // Java: com/google/devtools/build/lib/runtime/ProcessWrapper.java:177 CommandLineBuilder.build
    // Java: com/google/devtools/build/lib/runtime/ProcessWrapper.java:72 ProcessWrapper.fromCommandEnvironment
    #[test]
    fn execute_process_wrapper_command_line_follows_local_execution_options() {
        let args = || vec!["echo".to_owned(), "hello, world".to_owned()];
        // execute(timeout = 600) with --experimental_scale_timeouts=1.0
        let timeout_millis = 600_000;

        assert_eq!(
            process_wrapper_command_line(
                &process_wrapper("/install/process-wrapper", None, false),
                args(),
                timeout_millis
            ),
            vec![
                "/install/process-wrapper",
                "--timeout=600",
                "echo",
                "hello, world"
            ]
        );
        assert_eq!(
            process_wrapper_command_line(
                &process_wrapper(
                    "/install/process-wrapper",
                    Some(Duration::from_secs(15)),
                    false
                ),
                args(),
                timeout_millis
            ),
            vec![
                "/install/process-wrapper",
                "--timeout=600",
                "--kill_delay=15",
                "echo",
                "hello, world"
            ]
        );
        assert_eq!(
            process_wrapper_command_line(
                &process_wrapper(
                    "/install/process-wrapper",
                    Some(Duration::from_secs(15)),
                    true
                ),
                args(),
                timeout_millis
            ),
            vec![
                "/install/process-wrapper",
                "--timeout=600",
                "--kill_delay=15",
                "--graceful_sigterm",
                "echo",
                "hello, world"
            ]
        );
    }

    #[test]
    fn java_replace_all_expands_like_matcher() {
        assert_eq!(java_replace_all("a<x>b<x>", "<x>", "y"), "ayby");
        assert_eq!(java_replace_all("a<x>", "<x>", "\\$1"), "a$1");
        assert_eq!(java_replace_all("a<x>", "<x>", "[$0]"), "a[<x>]");
    }

    /// `credentials.getBytes(UTF_8)` of the internal string: each non-ASCII byte is encoded
    /// twice (`é` = C3 A9 becomes C3 83 C2 A9), a lone FF becomes C3 BF.
    #[test]
    fn basic_auth_value_encodes_internal_string_as_utf8() {
        assert_eq!(basic_auth_value("a:b"), "Basic YTpi");
        let credentials = format!(
            "{}:{}\u{ff}",
            latin1_decode("é".as_bytes()),
            latin1_decode("🐷".as_bytes())
        );
        let expected = base64::engine::general_purpose::STANDARD.encode([
            0xC3, 0x83, 0xC2, 0xA9, b':', 0xC3, 0xB0, 0xC2, 0x9F, 0xC2, 0x90, 0xC2, 0xB7, 0xC3,
            0xBF,
        ]);
        assert_eq!(basic_auth_value(&credentials), format!("Basic {expected}"));
    }
}
