//! `BlazeCommandDispatcher`: dispatches one command to its `BlazeCommand`, after parsing its
//! options (rc files, `--config`) and setting up the event handler.

use std::sync::Arc;
use std::time::{Duration, Instant};

use blaze_events::ui::{UiEventHandler, UiEventHandlerOptions};
use blaze_events::{Event, EventHandler, EventKind, EventKindSet, OutErr};
use blaze_optdefs::command::common_command_options::CommonCommandOptions;
use blaze_optdefs::command::ui_options::UiOptions;
use blaze_options::invocation_policy::OptionAndRawValue;
use blaze_options::rc::handler::{
    BAD_OPTION_TAG, BlazeOptionHandler, ERROR_SEPARATOR, EventKind as HandlerEventKind,
    HandlerEvent, OptionsParseFailure, StoredEventHandler,
};
use blaze_options::{OptionsParser, OptionsParsingException, PriorityCategory};
use blaze_protos::blaze::invocation_policy::InvocationPolicy;
use blaze_protos::failure_details::{self, FailureDetail};
use blaze_protos::google::protobuf::Any;
use blaze_server::command_dispatcher::{
    CommandDispatcher, CommandExtensionReporter, IdleTaskResultsSupplier,
};
use blaze_server::interruptible::Monitor;
use blaze_shell::thread_interrupt::InterruptedException;
use blaze_sky_loading::package_manager::RepositoryMappingResolutionOrInterrupted;
use parking_lot::Mutex;

use crate::command::{BlazeCommand, BlazeCommandResult};
use crate::command_environment::CommandEnvironment;
use crate::exit_code::{DetailedExitCode, ExitCode};
use crate::runtime::BlazeRuntime;
use crate::starlark_options_parser::{
    SkyframeExecutorTargetLoader, StarlarkOptionsParser, StarlarkOptionsParsingError,
};

/// What to do if the command lock is not available, and how much output to emit on the
/// console: `blaze_server`'s ports of `CommandDispatcher.LockingMode` and
/// `CommandDispatcher.UiVerbosity`.
///
/// PORT: Java's `CommandDispatcher` is in `lib/runtime`; it lives in `blaze_server`, below this
/// crate (decision D3), and its enums are re-exported here so that no caller changes.
// Java: com/google/devtools/build/lib/runtime/CommandDispatcher.java:35 CommandDispatcher.LockingMode
// Java: com/google/devtools/build/lib/runtime/CommandDispatcher.java:41 CommandDispatcher.UiVerbosity
pub use blaze_server::{LockingMode, UiVerbosity};

/// Dispatches to the Blaze commands; that is, given a command line, this abstraction looks up
/// the appropriate command object, parses the options required by the object, and calls its
/// exec method.
// Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:106 BlazeCommandDispatcher
pub struct BlazeCommandDispatcher {
    runtime: Arc<BlazeRuntime>,
    server_pid: i32,
    /// `commandLock` guarding `currentClientDescription`: the description of the client whose
    /// command holds the command lock.
    current_client_description: Monitor<Option<String>>,
    /// `shutdownReason` (an `AtomicReference`), shared with the running command's crash
    /// handling (`CommandEnvironment.shutdownReasonConsumer`, see
    /// [`BlazeRuntime::before_command`]).
    shutdown_reason: Arc<Mutex<Option<String>>>,
    /// Set by [`Self::exec_exclusively`] when the command ended with `REMOTE_CACHE_EVICTED` and
    /// may be retried: Java's `RemoteCacheTransientErrorException`.
    remote_cache_transient_error: Mutex<bool>,
    /// Whether [`Self::exec`] releases the build arena (`blaze_util::build_arena`) when a
    /// command has ended: only the server that runs many commands in one process does
    /// ([`Self::release_build_arena_after_commands`]).
    release_build_arena: bool,
}

// Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:111 BlazeCommandDispatcher.HELP_COMMAND
const HELP_COMMAND: &[&str] = &["help"];

// Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:113 BlazeCommandDispatcher.ALL_HELP_OPTIONS
const ALL_HELP_OPTIONS: &[&str] = &["--help", "-help", "-h"];

impl BlazeCommandDispatcher {
    // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:109 BlazeCommandDispatcher.UNKNOWN_SERVER_PID
    pub const UNKNOWN_SERVER_PID: i32 = -1;

    // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:137 BlazeCommandDispatcher.<init>
    pub fn new(runtime: Arc<BlazeRuntime>, server_pid: i32) -> Self {
        BlazeCommandDispatcher {
            runtime,
            server_pid,
            current_client_description: Monitor::new(None),
            shutdown_reason: Arc::new(Mutex::new(None)),
            remote_cache_transient_error: Mutex::new(false),
            release_build_arena: false,
        }
    }

    /// Makes [`Self::exec`] release the build arena after every command that ended normally,
    /// while it still holds the command lock (see [`release_build_arena`]).
    ///
    /// PORT: Blaze only (decision D7). The persistent server (`BlazeRuntime.serverMain`) runs
    /// many commands in one process; batch mode and the in-process server run one and never
    /// release it.
    pub fn release_build_arena_after_commands(mut self) -> Self {
        self.release_build_arena = true;
        self
    }

    /// Executes a single command: the `exec` that batch mode and the in-process server call,
    /// with no idle task results (Java's batch mode passes `() -> ImmutableList.of()`).
    ///
    /// `Err` is Java's `InterruptedException`: the command was interrupted while it waited for
    /// the command lock ([`LockingMode::WAIT`]).
    // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:192 BlazeCommandDispatcher.exec
    pub fn exec(
        &self,
        invocation_policy: InvocationPolicy,
        args: Vec<String>,
        out_err: OutErr,
        locking_mode: LockingMode,
        ui_verbosity: UiVerbosity,
        client_description: &str,
        startup_options_tagged_with_bazel_rc: Option<Vec<(String, String)>>,
    ) -> Result<BlazeCommandResult, InterruptedException> {
        self.exec_internal(
            invocation_policy,
            args,
            out_err,
            locking_mode,
            ui_verbosity,
            client_description,
            {
                use blaze_events::Clock;
                blaze_events::JavaClock.current_time_millis()
            },
            startup_options_tagged_with_bazel_rc,
            &|| Some(Vec::new()),
        )
    }

    /// Executes a single command.
    ///
    /// PORT: the idle task results of the previous idle period are taken from
    /// `idle_task_results_supplier` after the lock, as in Java, and dropped: Java hands them to
    /// the `CommandEnvironment` (`getIdleTaskResultsFromPreviousIdlePeriod`), which nothing
    /// ported reads. Command extensions and their reporter belong to the gRPC request; no ported
    /// command reads or reports any. The terminal size monitor only serves the cursor-control
    /// UI.
    // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:192 BlazeCommandDispatcher.exec
    fn exec_internal(
        &self,
        invocation_policy: InvocationPolicy,
        args: Vec<String>,
        out_err: OutErr,
        locking_mode: LockingMode,
        ui_verbosity: UiVerbosity,
        client_description: &str,
        first_contact_time: i64,
        startup_options_tagged_with_bazel_rc: Option<Vec<(String, String)>>,
        idle_task_results_supplier: &(dyn Fn() -> Option<Vec<blaze_server::IdleTaskResult>> + Sync),
    ) -> Result<BlazeCommandResult, InterruptedException> {
        // Default to help command if no arguments specified.
        let args: Vec<String> = if args.is_empty() {
            HELP_COMMAND.iter().map(|s| s.to_string()).collect()
        } else {
            args
        };

        let mut command_name = args[0].clone();

        // Be gentle to users who want to find out about Blaze invocation.
        if ALL_HELP_OPTIONS.contains(&command_name.as_str()) {
            command_name = "help".to_string();
        }

        let Some(command) = self.runtime.get_command_map().get(&command_name).cloned() else {
            out_err.print_err_ln(&format!(
                "Command '{}' not found. Try '{} help'.",
                command_name,
                self.runtime.get_product_name()
            ));
            return Ok(create_detailed_command_result(
                &format!("Command '{}' not found.", command_name),
                failure_details::command::Code::CommandNotFound,
            ));
        };

        // Take the exclusive server lock.  If we fail, we busy-wait until the lock becomes
        // available.
        //
        // We used to rely on commandLock.wait() to lazy-wait for the lock to become available,
        // which is theoretically fine, but doing so prevents us from determining if the PID of
        // the server holding the lock has changed under the hood.  There have been multiple bug
        // reports where users (especially macOS ones) mention that the Blaze invocation hangs on
        // a non-existent PID. This should help troubleshoot those scenarios in case there really
        // is a bug somewhere.
        let mut multiple_attempts = false;
        let clock_before = Instant::now();
        let mut other_client_description = String::new();
        {
            let mut current_client_description = self.current_client_description.lock();
            while let Some(current) = current_client_description.clone() {
                match locking_mode {
                    LockingMode::WAIT => {
                        if other_client_description != current {
                            let server_description = if self.server_pid == Self::UNKNOWN_SERVER_PID
                            {
                                String::new()
                            } else {
                                format!(" (server_pid={})", self.server_pid)
                            };
                            out_err.print_err_ln(&format!(
                                "Another command ({}) is running. Waiting for it to complete on \
                                 the server{}...",
                                current, server_description
                            ));
                            other_client_description = current;
                        }
                        // commandLock.wait(500): an interrupt of this thread (a Cancel of this
                        // command) ends the wait with InterruptedException.
                        self.current_client_description.wait(
                            &mut current_client_description,
                            Some(Duration::from_millis(500)),
                        )?;
                    }
                    LockingMode::ERROR_OUT => {
                        let message = format!(
                            "Another command ({}) is running. Exiting immediately.",
                            current
                        );
                        out_err.print_err_ln(&message);
                        return Ok(create_detailed_command_result(
                            &message,
                            failure_details::command::Code::AnotherCommandRunning,
                        ));
                    }
                }

                multiple_attempts = true;
            }
            *current_client_description = Some(client_description.to_string());
        }
        // finally { synchronized (commandLock) { currentClientDescription = null;
        // commandLock.notify(); } }: also when a panic unwinds out of this method, so that a
        // crash never leaves the lock held.
        let _command_lock = CommandLockRelease(&self.current_client_description);
        // If we took the lock on the first try, force the reported wait time to 0 to avoid
        // unnecessary noise in the logs.  In this metric, we are only interested in knowing how
        // long it took for other commands to complete, not how fast acquiring a lock is.
        let wait_time_in_ms: i64 = if !multiple_attempts {
            0
        } else {
            clock_before.elapsed().as_millis() as i64
        };

        // Retrieve information about idle tasks that ran during a previous idle period.
        // We do this after obtaining the lock so that a non-blocking command doesn't cause this
        // information to be lost (instead, it will be forwarded to the next command).
        let _idle_task_results_from_previous_idle_period = idle_task_results_supplier();

        let result = (|| {
            if let Some(reason) = self.shutdown_reason.lock().clone() {
                out_err.print_err_ln(&reason);
                return create_detailed_command_result(
                    &reason,
                    failure_details::command::Code::PreviouslyShutdown,
                );
            }
            // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:290 BlazeCommandDispatcher.exec
            let mut attempt_number = 0;
            let mut attempted_commands = 0;
            // String buildRequestIdOverride = null (kept by crate::commands::build_remote, which
            // computes the build request id).
            crate::commands::build::build_remote::reset_build_request_id_override();
            let result = loop {
                attempt_number += 1;
                *self.remote_cache_transient_error.lock() = false;
                // Java: execExclusively's `catch (Throwable e)` (BlazeCommandDispatcher.java:862):
                // a crash is handled and the command ends with the crash's exit code.
                // PORT: caught here, around the call; the panic unwinds out of the command (no
                // afterCommand, as Java's `needToCallAfterCommand = false`). Java's try starts
                // after the CommandEnvironment is created; a crash before that reaches
                // BlazeRuntime.main, whose handling prints and exits the same way.
                let exec = crate::bugreport::catch_crash(&args, || {
                    self.exec_exclusively(
                        &invocation_policy,
                        &args,
                        out_err.clone(),
                        ui_verbosity == UiVerbosity::QUIET,
                        &command_name,
                        command.clone(),
                        wait_time_in_ms,
                        first_contact_time,
                        startup_options_tagged_with_bazel_rc.clone(),
                        attempt_number,
                        attempted_commands,
                    )
                });
                let result = match exec {
                    Ok(result) => result,
                    Err(result) => {
                        // execExclusively's finally also runs on a crash: stop and clear the
                        // profiler (the reporter is gone with the environment).
                        // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:871 BlazeCommandDispatcher.execExclusively
                        let _ = crate::profiler_init::stop_and_clear_current();
                        // We are crashing: the command's environment is gone.
                        self.runtime.clear_env();
                        // Swallow IOException, as we are already in a finally clause
                        let _ = out_err.get_output_stream().flush();
                        let _ = out_err.get_error_stream().flush();
                        result
                    }
                };
                // catch (RemoteCacheTransientErrorException e)
                if std::mem::take(&mut *self.remote_cache_transient_error.lock()) {
                    attempted_commands += 1;
                    // Use a fixed build request ID across cache eviction retries to tie together
                    // the individual invocations, which all have different invocation IDs.
                    crate::commands::build::build_remote::retry_with_build_request_id_of_last_attempt();
                    continue;
                }
                break result;
            };
            if result.shutdown() {
                self.set_shutdown_reason(format!(
                    "Server shut down {}",
                    if result.get_exit_code().is_infrastructure_failure() {
                        format!(
                            "due to a crash: {}",
                            result
                                .get_failure_detail()
                                .map(|f| f.message.as_str())
                                .unwrap_or("")
                        )
                    } else {
                        format!("explicitly by client {}", client_description)
                    }
                ));
            } else if self.release_build_arena {
                // The kept graph's last executor is gone with the command: its functions, and
                // the command's state they hold, go now (not with the next command's executor).
                let incremental = self.runtime.get_workspace().get_incremental_state();
                incremental.release_command_functions();
                release_build_arena(incremental.has_kept_graph());
            }
            result
        })();
        Ok(result)
    }

    /// Shuts down all the registered commands to give them a chance to cleanup or close
    /// resources. Should be called by the owner of this command dispatcher in all termination
    /// cases.
    ///
    /// PORT: Java closes `logOutputStream`, which nothing assigns at the pin (always null).
    // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:1059 BlazeCommandDispatcher.shutdown
    pub fn shutdown(&self) {}

    // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:363 BlazeCommandDispatcher.execExclusively
    fn exec_exclusively(
        &self,
        invocation_policy: &InvocationPolicy,
        args: &[String],
        out_err: OutErr,
        quiet: bool,
        _command_name: &str,
        command: Arc<dyn BlazeCommand>,
        wait_time_in_ms: i64,
        first_contact_time: i64,
        startup_options_tagged_with_bazel_rc: Option<Vec<(String, String)>>,
        attempt_number: i32,
        attempted_commands: i32,
    ) -> BlazeCommandResult {
        // Record the start time for the profiler. Do not put anything before this!
        // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:382 BlazeCommandDispatcher.execExclusively
        let exec_start_time_nanos = {
            use blaze_events::Clock;
            blaze_events::JavaClock.nano_time()
        };
        let command_annotation = command.annotation();
        let workspace = self.runtime.get_workspace().clone();

        let mut stored_event_handler = StoredEventHandler::default();
        let mut options_parser = self.create_options_parser(command.as_ref());

        // BlazeOptionHandler.parseOptionsAndGetConfigDefinitions, preceded by its
        // checkCwdInWorkspace (Java does both inside parseOptionsInternal).
        let all_options_classes = self.runtime.get_all_command_options_classes();
        let valid_commands: Vec<String> = self.runtime.get_command_map().keys().cloned().collect();
        let mut config_flag_definitions =
            blaze_sky_analysis::flag_set_function::ConfigFlagDefinitions::default();
        let mut early_exit_code: Option<DetailedExitCode> =
            Some(self.check_cwd_in_workspace(command.as_ref(), &mut stored_event_handler));
        let rcfile_notes: Vec<String>;
        {
            let edit_command = command.clone();
            let mut option_handler = BlazeOptionHandler::new(
                self.runtime.get_product_name(),
                &command_annotation.rc,
                valid_commands,
                all_options_classes,
                &mut options_parser,
            )
            .with_invocation_policy(
                self.runtime.get_module_invocation_policy(),
                invocation_policy.clone(),
                // Allow the command to edit the options (Java: inside parseOptionsInternal,
                // after parseArgsAndConfigs succeeded, before the policy is enforced).
                Box::new(move |parser: &mut OptionsParser| edit_command.edit_options(parser)),
            );
            if early_exit_code.as_ref().unwrap().is_success() {
                // `invocationPolicyFlagListBuilder= ImmutableList.builder()`: the first pass's
                // list is dropped.
                let (result, definitions) = option_handler
                    .parse_options_and_get_config_definitions(args, &mut stored_event_handler);
                early_exit_code = Some(options_parse_result_exit_code(result));
                config_flag_definitions =
                    crate::project_options::config_flag_definitions(&definitions);
            }
            rcfile_notes = option_handler.get_rcfile_notes().to_vec();
        }
        let options = Arc::new(options_parser);

        let mut command_env_warnings = Vec::new();
        let mut env = CommandEnvironment::new(
            self.runtime.clone(),
            workspace.clone(),
            command_annotation,
            options.clone(),
            invocation_policy.clone(),
            &mut command_env_warnings,
            wait_time_in_ms,
            first_contact_time,
            attempt_number,
        );
        env.set_client_command_line(args.to_vec(), startup_options_tagged_with_bazel_rc.clone());
        env.set_config_flag_definitions(config_flag_definitions);
        // BlazeWorkspace.initCommand: "Drop the action cache reference to save memory since we
        // don't need it for this build. If a subsequent build needs it,
        // getOrLoadPersistentActionCache will reload it from disk."
        // Java: com/google/devtools/build/lib/runtime/BlazeWorkspace.java:348 BlazeWorkspace.initCommand
        if options
            .get_options::<blaze_optdefs::command::build_request_options::BuildRequestOptions>()
            .is_some_and(|o| !o.use_action_cache)
        {
            crate::idle_tasks::note_persistent_action_cache_dropped();
        }

        // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:419 BlazeCommandDispatcher.execExclusively
        if attempt_number > 1 {
            out_err.print_err_ln("Found transient remote cache error, retrying the build...");
        }

        let common_options = options
            .get_options::<CommonCommandOptions>()
            .expect("CommonCommandOptions");
        // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:422 BlazeCommandDispatcher.execExclusively
        let tracer_enabled = crate::profiler_init::tracer_enabled(
            &common_options,
            command_annotation.name(),
            command_annotation.build_phase.analyzes(),
        );
        // TODO(ulfjack): Move the profiler initialization as early in the startup sequence as
        // possible. Profiler setup and shutdown must always happen in pairs. Shutdown is
        // currently performed in the afterCommand call in the finally block below.
        let profiler_started_event = crate::profiler_init::init_profiler(
            tracer_enabled,
            &mut stored_event_handler,
            &options,
            &env,
            exec_start_time_nanos,
            wait_time_in_ms,
            &crate::version_info::BlazeVersionInfo::instance().get_release_name(),
        );
        crate::profiler_init::profile_starting(profiler_started_event);
        // PORT: skipped, profiling: the Starlark CPU profiler (--starlark_cpu_profile) is not
        // started.

        let reporter = env.get_reporter().clone();
        // OutErr.SystemPatcher systemOutErrPatcher = reporter.getOutErr().getSystemPatcher();
        // closed by execExclusively's finally (here: when the guard is dropped, after
        // afterCommand and the flush of the command's streams, also when a panic unwinds).
        // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:485 BlazeCommandDispatcher.execExclusively
        let system_out_err_patcher =
            SystemPatcherClose(reporter.get_out_err().get_system_patcher());
        // Both the call to env.decideKeepIncrementalState() and module.beforeCommand() may emit
        // events, but the reporter isn't setup yet. Use a stored event handler to catch those
        // events.
        let module_events = Arc::new(blaze_events::StoredEventHandler::new());
        reporter.add_handler(module_events.clone());
        // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:490 BlazeCommandDispatcher.execExclusively
        env.decide_keep_incremental_state();
        // A BlazeModuleEnvironment.exit before beforeCommand stays pending: Java reports it from
        // CommandEnvironment.beforeCommand (throwPendingException).
        for module in self.runtime.get_blaze_modules().to_vec() {
            // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:492 BlazeCommandDispatcher.execExclusively
            let _closeable = blaze_util::profiler::Profiler::instance()
                .profile_with(blaze_util::profiler::ProfilerTask::INFO, || {
                    format!("{}.beforeCommand", module.name())
                });
            if let Err(e) = module.before_command(&mut env) {
                // Don't let one module's complaints prevent the other modules from doing
                // necessary setup.
                module_events.handle(&Event::error(e.get_message()));
                early_exit_code = DetailedExitCode::choose_more_important_with_first_if_tie(
                    early_exit_code,
                    Some(e.get_detailed_exit_code().clone()),
                );
            }
        }
        let module_events_handler: Arc<dyn EventHandler> = module_events.clone();
        reporter.remove_handler(&module_events_handler);

        // Setup stdout / stderr.
        // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:510 BlazeCommandDispatcher.execExclusively
        let out_err = tee(out_err, env.get_output_listeners());

        let early_exit_code = early_exit_code.unwrap();
        // Early exit. We need to guarantee that the ErrOut and Reporter setup below never error
        // out, so any invariants they need must be checked before this point.
        if !early_exit_code.is_success() {
            replay_early_exit_events(
                &out_err,
                &rcfile_notes,
                &stored_event_handler,
                &module_events,
            );
            post_no_build_event(&env);
            let result = BlazeCommandResult::detailed_exit_code(early_exit_code);
            return self.after_command_on_early_exit(&mut env, result, &out_err);
        }

        // Setup the event handler.
        // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:526 BlazeCommandDispatcher.execExclusively
        let setup_event_handler =
            blaze_util::profiler::Profiler::instance().profile_info("setup event handler");
        let event_handler_options = options.get_options::<UiOptions>().expect("UiOptions");
        let mut out_err = out_err;
        if !event_handler_options.use_color() {
            if !command_annotation.binary_std_out {
                out_err = blaze_events::ui::ansi_strip_out(&out_err);
            }
            if !command_annotation.binary_std_err {
                out_err = blaze_events::ui::ansi_strip_err(&out_err);
            }
        }
        // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:540 BlazeCommandDispatcher.execExclusively
        if !command_annotation.binary_std_out {
            out_err = buffer_out(&out_err);
        }
        if !command_annotation.binary_std_err {
            out_err = buffer_err(&out_err);
        }
        // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:549 BlazeCommandDispatcher.execExclusively
        crate::server_log::setup_logging(common_options.verbosity);
        inject_panic_on_detached_thread_if_requested(command_annotation.name());
        let handler = Arc::new(UiEventHandler::new(
            out_err.clone(),
            &ui_event_handler_options(&event_handler_options),
            quiet,
            Arc::new(blaze_events::ui::JavaClock),
            self.runtime
                .get_workspace()
                .get_directories()
                .get_workspace()
                .map(|w| w.get_path_string().to_string()),
        ));
        // UiEventHandler.<init>: the state tracker. The build commands replace it with a
        // SkymeldUiStateTracker when they merge analysis and execution
        // (env.withMergedAnalysisAndExecutionSourceOfTruth(), decided in build.rs).
        // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:241 UiEventHandler.UiEventHandler
        // newStatsSummary: ExecutionOptions.statsSummary.
        let new_stats_summary = options
            .get_options::<blaze_optdefs::command::execution_options::ExecutionOptions>()
            .is_some_and(|o| o.stats_summary);
        let ui_state_tracker = Arc::new(new_ui_state_tracker(
            &handler,
            &event_handler_options,
            false,
            new_stats_summary,
        ));
        handler.set_state_tracker(ui_state_tracker.clone());
        let ui_subscriber = Arc::new(
            blaze_buildtool::runtime::ui_event_subscriber::UiEventSubscriber::new(
                handler.clone(),
                ui_state_tracker,
            ),
        );
        // PORT: the UI handler is registered behind a filter that drops the events of the main
        // repository mapping once when the command's own evaluation reports them again (see
        // MainRepoMappingEventHandler).
        let ui_filter = Arc::new(ReportedOnceFilter::new(handler.clone()));
        reporter.add_handler(ui_filter.clone());
        env.set_ui_event_handler(handler.clone(), ui_subscriber);
        // PORT: skipped, registered only with the ANSI-allowing handler: PassiveExperimentalEventHandler.noBuild
        // PORT: skipped, registered only with the ANSI-allowing handler: PassiveExperimentalEventHandler.noBuildFinished
        // PORT: the ANSI-allowing handler for --color=no (registerAnsiAllowingHandler, and its
        // PassiveExperimentalEventHandler on the event bus) is not registered, so
        // Reporter.switchToAnsiAllowingHandler (info, run) keeps the UI handler. Java's
        // ANSI-allowing handler is a second UiEventHandler over `colorfulOutErr`, which is
        // ANSI-stripped exactly like `outErr` (only not buffered).

        drop(setup_event_handler);

        // PORT: warnIfUsingUnusupportedEncoding: the in-process server's sun.jnu.encoding is the
        // ISO-8859-1 the client forces (see InfoItem character-encoding), which is supported.

        // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:579 BlazeCommandDispatcher.execExclusively
        let replay_stored_events =
            blaze_util::profiler::Profiler::instance().profile_info("replay stored events");
        // Now we're ready to replay the events.
        for event in stored_event_handler.get_events() {
            reporter.handle(&to_event(event));
        }
        module_events.replay_on(reporter.as_ref());
        for warning in &command_env_warnings {
            reporter.handle(&Event::warn(warning.clone()));
        }

        drop(replay_stored_events);

        // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:587 BlazeCommandDispatcher.execExclusively
        let announce_rc_options =
            blaze_util::profiler::Profiler::instance().profile_info("announce rc options");
        if common_options.announce_rc_options {
            if let Some(tagged) = &startup_options_tagged_with_bazel_rc {
                let mut last_blazerc = String::new();
                let mut accumulated_startup_options: Vec<String> = Vec::new();
                for (source, option) in tagged {
                    // Do not include the command line options, marked by the empty string.
                    if source.is_empty() {
                        continue;
                    }
                    // If we've moved to a new blazerc in the list, print out the info from the
                    // last one, and clear the accumulated list.
                    if !last_blazerc.is_empty() && *source != last_blazerc {
                        reporter.handle(&Event::info(format!(
                            "Reading 'startup' options from {}: {}",
                            last_blazerc,
                            accumulated_startup_options.join(", ")
                        )));
                        accumulated_startup_options.clear();
                    }
                    last_blazerc = source.clone();
                    accumulated_startup_options.push(option.clone());
                }
                // Print out the final blazerc-grouped list, if any startup options were provided
                // by blazerc.
                if !last_blazerc.is_empty() {
                    reporter.handle(&Event::info(format!(
                        "Reading 'startup' options from {}: {}",
                        last_blazerc,
                        accumulated_startup_options.join(", ")
                    )));
                }
            }
            for note in &rcfile_notes {
                reporter.handle(&Event::info(note.clone()));
            }
        }

        drop(announce_rc_options);

        // While a Blaze command is active, direct all errors to the client's event handler (and
        // out/err streams).
        // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:630 BlazeCommandDispatcher.execExclusively
        system_out_err_patcher.0.start();

        // Notify the BlazeRuntime, so it can do some initial setup.
        // Java: CommandEnvironment.beforeCommand
        // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:632 BlazeCommandDispatcher.execExclusively
        let closeable =
            blaze_util::profiler::Profiler::instance().profile_info("CommandEnv.beforeCommand");
        let env_before_command = self.env_before_command(&mut env);
        drop(closeable);
        if let Err(e) = env_before_command {
            reporter.handle(&Event::error(e.get_message()));
            let result = BlazeCommandResult::detailed_exit_code(e.get_detailed_exit_code().clone());
            return self.after_command_on_early_exit(&mut env, result, &out_err);
        }

        let mut options = options;
        // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:649 BlazeCommandDispatcher.execExclusively
        if command_annotation.build_phase.analyzes() {
            // env.syncPackageLoading(options)
            // PORT: Blaze has no SkyframeExecutor that outlives a command; syncing package
            // loading is creating the command's executor
            // (`crate::commands::query::create_skyframe_executor`), which reports its own errors.
            // The commands create their own executor again in `exec`.
            // PORT: Java computes the main repository mapping on the executor the command then
            // uses, so its events (MODULE.bazel warnings, fetches) are reported once, here.
            // Blaze's command creates a new executor that evaluates them again; this handler
            // reports the events now and has the UI filter drop them once when the command
            // reports them again (see MainRepoMappingEventHandler).
            let stored_events = Arc::new(MainRepoMappingEventHandler::new(
                reporter.clone(),
                ui_filter.clone(),
            ));
            crate::commands::query::prepare_package_loading();
            let skyframe_executor =
                match crate::commands::query::create_skyframe_executor_reporting_to(
                    &env,
                    env.get_options().clone(),
                    None,
                    None,
                    stored_events.clone(),
                ) {
                    Ok(skyframe_executor) => skyframe_executor,
                    Err(result) => {
                        post_no_build_event(&env);
                        return self.after_command_on_early_exit(&mut env, result, &out_err);
                    }
                };

            // TODO: b/453809359 - Remove this Python logic when Bazel 9+ can read Python flag
            // alias definitions straight from rules_python's MODULE.bazel.
            // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:672 BlazeCommandDispatcher.execExclusively
            let remove_py_fragment =
                boolean_option_value(&options, "incompatible_remove_ctx_py_fragment");
            let remove_bazel_py_fragment =
                boolean_option_value(&options, "incompatible_remove_ctx_bazel_py_fragment");
            // Compute the repo mapping of the main repo and re-parse options so that we get correct
            // values for label-typed options.
            // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:701 BlazeCommandDispatcher.execExclusively
            // env.getEventBus().post(new MainRepoMappingComputationStartingEvent()): its only
            // subscriber is UiEventHandler.mainRepoMappingComputationStarted (an event of
            // lib/runtime, posted by calling the subscriber).
            if let Some(handler) = env.get_ui_event_handler() {
                handler.main_repo_mapping_computation_started();
            }
            let event_handler: Arc<dyn blaze_events::ExtendedEventHandler> = stored_events.clone();
            let interrupted = || {
                let message = "command interrupted while computing main repo mapping";
                stored_events.replay_on(env.get_reporter().as_ref());
                reporter.handle(&Event::error(message));
                interrupted_detailed_exit_code(message)
            };
            let mut early_exit_code = DetailedExitCode::success();
            let mut options_parser: Option<OptionsParser> = None;
            // getMainRepoMapping(eventHandler) is getMainRepoMapping(false, DEFAULT_THREAD_COUNT,
            // eventHandler).
            // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:702 BlazeCommandDispatcher.execExclusively
            let compute_main_repo_mapping = blaze_util::profiler::Profiler::instance().profile(
                blaze_util::profiler::ProfilerTask::BZLMOD,
                "compute main repo mapping",
            );
            match skyframe_executor.get_main_repo_mapping(
                false,
                blaze_sky_top::skyframe_executor::DEFAULT_THREAD_COUNT,
                event_handler.clone(),
            ) {
                Ok(main_repo_mapping) => {
                    let mut parser = options
                        .to_builder()
                        .with_conversion_context(Arc::new(main_repo_mapping))
                        .build()
                        .unwrap_or_else(|e| e.throw());
                    // Collect MODULE.bazel flag_alias(name = "foo", starlark_flag = "//bar")
                    // entries, so when builds set "--foo=1", that maps to "--//bar=1". Inject this
                    // as an implicit "--flag_alias=foo=//bar" flag. This is because select()s and
                    // configuration transitions really on that flag
                    // (CoreOptions.commandLineFlagAliases) to properly handle aliases.
                    match skyframe_executor.get_flag_aliases(
                        event_handler,
                        remove_py_fragment,
                        remove_bazel_py_fragment,
                    ) {
                        Ok(Some(flag_aliases)) => {
                            // PORT: Java has no handler for OptionsParsingException here; it
                            // reaches execExclusively's catch (Throwable), which crashes the
                            // server. Blaze panics.
                            parser
                                .parse_with_priority(
                                    PriorityCategory::RC_FILE,
                                    Some("module resolution"),
                                    blaze_build_config::flag_aliases::flag_alias_args(
                                        &flag_aliases,
                                    ),
                                )
                                .unwrap_or_else(|e| panic!("{}", e.get_message()));
                            options_parser = Some(parser);
                        }
                        // evalResult.get(BazelDepGraphValue.KEY) is null after the main repo
                        // mapping was computed.
                        Ok(None) => panic!(
                            "NullPointerException: BazelDepGraphValue after the main repo mapping"
                        ),
                        Err(_) => early_exit_code = interrupted(),
                    }
                }
                Err(RepositoryMappingResolutionOrInterrupted::Interrupted) => {
                    early_exit_code = interrupted();
                }
                Err(RepositoryMappingResolutionOrInterrupted::Resolution(e)) => {
                    stored_events.replay_on(env.get_reporter().as_ref());
                    reporter.handle(&Event::error(e.to_string()));
                    early_exit_code = crate::commands::query::to_runtime_detailed_exit_code(
                        blaze_query::compat::failure_details::from_packages(&e.detailed_exit_code),
                    );
                }
            }
            drop(compute_main_repo_mapping);
            if !early_exit_code.is_success() {
                post_no_build_event(&env);
                let result = BlazeCommandResult::detailed_exit_code(early_exit_code);
                return self.after_command_on_early_exit(&mut env, result, &out_err);
            }
            let mut options_parser = options_parser.expect("the options parser was rebuilt");
            // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:739 BlazeCommandDispatcher.execExclusively
            let reparse_options = blaze_util::profiler::Profiler::instance().profile(
                blaze_util::profiler::ProfilerTask::BZLMOD,
                "reparse options with main repo mapping",
            );
            let mut stored_second_pass_event_handler = StoredEventHandler::default();
            let (early_exit_code, invocation_policy_flags) = self.parse_options(
                &command,
                invocation_policy,
                args,
                &mut options_parser,
                &mut stored_second_pass_event_handler,
            );
            if !early_exit_code.is_success() {
                for event in stored_second_pass_event_handler.get_events() {
                    reporter.handle(&to_event(event));
                }
            }
            env.set_invocation_policy_flags(invocation_policy_flags);
            drop(reparse_options);
            if !early_exit_code.is_success() {
                post_no_build_event(&env);
                let result = BlazeCommandResult::detailed_exit_code(early_exit_code);
                return self.after_command_on_early_exit(&mut env, result, &out_err);
            }

            // Parse starlark options.
            // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:768 BlazeCommandDispatcher.execExclusively
            // PORT: Java calls optionHandler.parseStarlarkOptions(env, args) for every command;
            // it returns success at once unless the command analyzes ("For now, restrict
            // starlark options to commands that already build to ensure that loading will
            // work"), which is this branch. The build settings are loaded on the executor that
            // computed the main repository mapping; like that computation, its events are
            // stored and reported only if the command stops here (the command's own executor
            // reports them when it loads the same packages).
            // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:766 BlazeCommandDispatcher.execExclusively
            let parse_starlark_options_scope = blaze_util::profiler::Profiler::instance().profile(
                blaze_util::profiler::ProfilerTask::BZLMOD,
                "parse starlark options",
            );
            let early_exit_code = parse_starlark_options(
                &env,
                &skyframe_executor,
                &stored_events,
                &mut options_parser,
                args,
            );
            drop(parse_starlark_options_scope);
            drop(skyframe_executor);
            if early_exit_code.is_success()
                && env
                    .get_blaze_workspace()
                    .get_incremental_state()
                    .passes_graph_between_executors()
            {
                // The command's executor builds on this executor's graph and finds these nodes
                // done, as Java's one executor does: it reports and posts nothing for them. The
                // posts go out now, and nothing is to be dropped from the command's output.
                stored_events.replay_on(env.get_reporter().as_ref());
                ui_filter.forget_reported();
            }
            if !early_exit_code.is_success() {
                post_no_build_event(&env);
                let result = BlazeCommandResult::detailed_exit_code(early_exit_code);
                return self.after_command_on_early_exit(&mut env, result, &out_err);
            }
            // optionHandler.getOptionsResult(). Like Java, env.getOptions() keeps the first
            // parse; the command gets this one.
            options = Arc::new(options_parser);
        }
        // From here on the events of the main repository mapping were reported (Java: the
        // command's evaluations find those nodes done and report nothing for them).
        ui_filter.start_dropping();

        // Log the command line now that the modules have all had a change to register their
        // listeners to the event bus, and the flags have been re-parsed.
        // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:779 BlazeCommandDispatcher.execExclusively
        if let Some(bep) = env.get_bep() {
            bep.post_command_line_events(&env, &options);
        }

        // PORT: the options of module hooks that are not ported fail loudly here, where the
        // options are final (see crate::unported_options).
        crate::unported_options::check_unported_options(command_annotation, &options);

        // Run the command.
        let mut result = command.exec(&mut env, &options);

        // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:828 BlazeCommandDispatcher.execExclusively
        let module_exit_code = env.precomplete_command(result.get_detailed_exit_code());
        // If Blaze did not suffer an infrastructure failure, check for errors in modules.
        if !result.get_exit_code().is_infrastructure_failure()
            && let Some(module_exit_code) = module_exit_code
        {
            result = BlazeCommandResult::detailed_exit_code(module_exit_code);
        }
        // The CommandPrecompleteEvent is posted at the start of after_command (BlazeRuntime
        // .afterCommand); the commands that build post it earlier, before their remote module's
        // afterCommand (crate::commands::build::run_build).
        let new_result = self.after_command(&mut env, result, &out_err);
        // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:851 BlazeCommandDispatcher.execExclusively
        if new_result.get_exit_code() == ExitCode::REMOTE_CACHE_EVICTED {
            let execution_options = options
                .get_options::<blaze_optdefs::command::execution_options::ExecutionOptions>()
                .expect("NullPointerException: ExecutionOptions");
            if attempted_commands < execution_options.remote_retry_on_transient_cache_error {
                // throw new RemoteCacheTransientErrorException(...)
                *self.remote_cache_transient_error.lock() = true;
            }
        }
        new_result
    }

    /// `CommandEnvironment.beforeCommand`.
    /// PORT: posts BuildMetadataEvent and GotOptionsEvent (whose only subscriber is the BEP's
    /// streamer), throws the pending module exception and runs the runtime hook. No Bazel module provides an
    /// output service besides the remote module's (the remote output service fails loudly in
    /// crate::unported_options). PORT: skipped, incremental: the timestamp granularity monitor,
    /// the client environment and package path synced into the server's SkyframeExecutor
    /// (Blaze's commands create their executor from the same options).
    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:919 CommandEnvironment.beforeCommand
    fn env_before_command(
        &self,
        env: &mut CommandEnvironment,
    ) -> Result<(), crate::module::AbruptExitException> {
        if let Some(bep) = env.get_bep() {
            bep.post_before_command_events(env);
        }
        // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:924 CommandEnvironment.beforeCommand
        env.throw_pending_exception()?;
        // CommandEnvironment's shutdownReasonConsumer (this::setShutdownReason) and command
        // thread, for notifyOnCrash.
        let shutdown_reason = self.shutdown_reason.clone();
        self.runtime.before_command(
            env,
            Arc::new(move |reason: String| set_shutdown_reason(&shutdown_reason, reason)),
        );
        // RemoteModule.beforeCommand for the commands that do not set the remote module up
        // themselves (crate::commands::build::build_remote::dispatcher_before_command).
        crate::commands::build::build_remote::dispatcher_before_command(env)?;
        Ok(())
    }

    /// An early `return result` of execExclusively: its `finally` block calls `afterCommand`
    /// (`needToCallAfterCommand` is still true) and only logs `afterCommand`'s result when it
    /// differs; the command's result is `result`.
    // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:884 BlazeCommandDispatcher.execExclusively
    fn after_command_on_early_exit(
        &self,
        env: &mut CommandEnvironment,
        result: BlazeCommandResult,
        out_err: &OutErr,
    ) -> BlazeCommandResult {
        // PORT: skipped, logging ("afterCommand yielded different result").
        let _new_result = self.after_command(env, result.clone(), out_err);
        result
    }

    /// `BlazeRuntime.afterCommand` plus the stream flushing of execExclusively's finally block.
    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:750 BlazeRuntime.afterCommand
    fn after_command(
        &self,
        env: &mut CommandEnvironment,
        command_result: BlazeCommandResult,
        out_err: &OutErr,
    ) -> BlazeCommandResult {
        // execExclusively's finally, before it calls afterCommand: profiler might still be
        // running when an exception is thrown before BuildCompleteEvent is emitted or
        // BlazeModule#completeCommand() is called. So we still need to try to stop the profiler
        // here.
        // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:871 BlazeCommandDispatcher.execExclusively
        if let Some(message) = crate::profiler_init::stop_and_clear_current() {
            env.get_reporter().handle(&Event::error(message));
        }
        // this.env = null
        self.runtime.clear_env();

        // Remove any filters that the command might have added to the reporter.
        // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:756 BlazeRuntime.afterCommand
        env.get_reporter()
            .set_output_filter(blaze_events::output_filter::output_everything());

        let mut module_exit_code: Option<DetailedExitCode> = None;
        // Ensure deterministic ordering of doing the metrics upload before everything else that
        // happens when BuildCompleteEvent is posted.
        // env.getEventBus().post(new CommandPrecompleteEvent()): its only subscriber with an
        // effect is the MetricsCollector, which lives with the BEP.
        // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:772 BlazeRuntime.afterCommand
        if let Some(bep) = env.get_bep() {
            bep.command_precomplete();
        }
        // RemoteModule.afterCommand, for every command: the repository helpers and the overlay
        // file system of the remote repo contents cache.
        crate::commands::build::build_remote::reset_repo_helpers_and_overlay_fs(&env);
        // RemoteModule.afterCommand of a command set up by env_before_command
        // (crate::commands::build::build_remote::dispatcher_after_command).
        if let Some(e) = crate::commands::build::build_remote::dispatcher_after_command() {
            env.get_reporter().handle(&Event::error(e.get_message()));
            module_exit_code = DetailedExitCode::choose_more_important_with_first_if_tie(
                module_exit_code,
                Some(e.get_detailed_exit_code().clone()),
            );
        }
        for module in self.runtime.get_blaze_modules() {
            if let Err(e) = module.after_command() {
                env.get_reporter().handle(&Event::error(e.get_message()));
                module_exit_code = DetailedExitCode::choose_more_important_with_first_if_tie(
                    module_exit_code,
                    Some(e.get_detailed_exit_code().clone()),
                );
            }
        }
        // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:795 BlazeRuntime.afterCommand
        let mut final_command_result = match module_exit_code {
            Some(code) if !command_result.get_exit_code().is_infrastructure_failure() => {
                match command_result.get_exec_request() {
                    Some(exec_request) => BlazeCommandResult::execute_with_detailed_exit_code(
                        exec_request.clone(),
                        code,
                    ),
                    None => BlazeCommandResult::detailed_exit_code(code),
                }
            }
            _ => command_result,
        };
        // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:809 BlazeRuntime.afterCommand
        if let Some(other_thread_won_exit_code) = self
            .runtime
            .notify_command_complete(env, final_command_result.get_detailed_exit_code())
        {
            final_command_result =
                BlazeCommandResult::detailed_exit_code(other_thread_won_exit_code);
        }
        // env.getBlazeWorkspace().clearEventBus(): "EventBus does not have an unregister()
        // method, so this is how we release memory associated with handlers" (the workspace's
        // SkyframeExecutor drops its reference to the command's event bus).
        // PORT: Blaze's Skyframe executors belong to the command and are gone by now; the
        // memory of the handlers is released below, where the last reference Java keeps (the
        // reporter's) is cleared.
        // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:810 BlazeRuntime.afterCommand
        if let Some(handler) = env.get_ui_event_handler() {
            // UiEventHandler is subscribed to AfterCommandEvent / BuildComplete; it flushes its
            // buffers there.
            handler.after_command();
        }
        // Some module's commandComplete() relies on the stoppage of profiler (stopped above). And
        // it is impossible the profiler is needed after all `BlazeModule.afterCommand`s are
        // executed.
        // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:821 BlazeRuntime.afterCommand
        for module in self.runtime.get_blaze_modules() {
            let _closeable = blaze_util::profiler::Profiler::instance()
                .profile_with(blaze_util::profiler::ProfilerTask::INFO, || {
                    format!("{}.commandComplete", module.name())
                });
            module.command_complete();
        }
        if let Some(reason) = env.take_shutdown_reason() {
            self.set_shutdown_reason(reason);
        }
        // Wipe the dependency graph if requested. Note that this method always runs at the end
        // of a commands unless the server crashes, in which case no inmemory state will linger
        // for the next build anyway.
        // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:790 BlazeRuntime.afterCommand
        if env
            .get_options()
            .get_options::<blaze_optdefs::command::common_command_options::CommonCommandOptions>()
            .is_some_and(|o| !o.keep_state_after_build)
        {
            env.get_blaze_workspace().reset_evaluator();
        }
        // Swallow IOException, as we are already in a finally clause
        let _ = out_err.get_output_stream().flush();
        let _ = out_err.get_error_stream().flush();
        // PORT: Java waits at the end of execExclusively's finally block, after afterCommand;
        // here it is the last step of the afterCommand port that every exit path runs.
        // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:899 BlazeCommandDispatcher.execExclusively
        env.get_timestamp_granularity_monitor()
            .wait_for_timestamp_granularity();
        // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:827 BlazeRuntime.afterCommand
        let idle_tasks = env.get_idle_tasks();
        if !idle_tasks.is_empty() {
            final_command_result =
                BlazeCommandResult::with_idle_tasks(final_command_result, idle_tasks);
        }
        // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:832 BlazeRuntime.afterCommand
        env.get_reporter().clear_event_bus();
        // PORT: with the reporter's and the workspace's references gone, Java's collector frees
        // the command's event bus and the subscribers registered on it. Blaze's subscribers can
        // hold the bus themselves (ExecutionProgressReceiverSetup, the test result aggregator),
        // so the bus and its subscribers would keep each other alive for the life of the server;
        // the bus drops its subscribers here instead.
        env.get_event_bus().clear_subscribers();
        // PORT: actionKeyContext.clear(): Blaze's ActionKeyContext belongs to the command's
        // Skyframe executor, which is gone.
        // storedExitCode.set(null)
        // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:834 BlazeRuntime.afterCommand
        crate::server_log::flush_server_log();
        self.runtime.clear_stored_exit_code();
        // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:836 BlazeRuntime.afterCommand
        BlazeCommandResult::with_response_extensions(
            final_command_result,
            env.get_response_extensions(),
        )
    }

    /// Creates an option parser using the common options classes and the command-specific
    /// options classes.
    // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:389 BlazeCommandDispatcher.createOptionsParser
    fn create_options_parser(&self, command: &dyn BlazeCommand) -> OptionsParser {
        let annotation = command.annotation();
        let options_classes =
            crate::command_registry::get_options(annotation, self.runtime.get_blaze_modules());
        let workspace_path = self.runtime.get_workspace().get_workspace();
        let has_module_dot_bazel =
            workspace_path.is_some_and(|w| w.get_relative_str("MODULE.bazel").exists());
        OptionsParser::builder()
            .options_classes(&options_classes)
            .skip_starlark_option_prefixes()
            .allow_residue(annotation.allow_residue)
            .with_alias_flag(Some(BLAZE_ALIASING_FLAG))
            .is_first_round_of_parsing(annotation.build_phase.analyzes() && has_module_dot_bazel)
            .build()
            .unwrap_or_else(|e| e.throw())
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeOptionHandler.java:642 BlazeOptionHandler.getNotInRealWorkspaceError
    fn get_not_in_real_workspace_error(&self, do_not_build_file: &blaze_vfs::Path) -> String {
        let product_name = self.runtime.get_product_name();
        let mut message =
            format!("{product_name} should not be called from a {product_name} output directory. ");
        // FileSystemUtils.readContentAsLatin1: one char per byte.
        let mut content = Vec::new();
        if let Ok(mut input) = do_not_build_file.get_input_stream() {
            if std::io::Read::read_to_end(&mut input, &mut content).is_ok() {
                let real_workspace = blaze_util::string_encoding::latin1_decode(&content);
                message.push_str(&format!(
                    "The pertinent workspace directory is: '{real_workspace}'"
                ));
            }
            // We are exiting anyway.
        }
        message
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeOptionHandler.java:156 BlazeOptionHandler.checkCwdInWorkspace
    fn check_cwd_in_workspace(
        &self,
        command: &dyn BlazeCommand,
        event_handler: &mut StoredEventHandler,
    ) -> DetailedExitCode {
        let command_annotation = command.annotation();
        if !command_annotation.must_run_in_workspace {
            return DetailedExitCode::success();
        }
        let workspace = self.runtime.get_workspace();
        if !workspace.get_directories().in_workspace() {
            let message = format!(
                "The '{}' command is only supported from within a workspace (below a directory \
                 having a MODULE.bazel file).\nSee documentation at \
                 https://bazel.build/concepts/build-ref#workspace",
                command_annotation.name()
            );
            event_handler.handle(HandlerEvent::error(message.clone()));
            return create_options_detailed_exit_code(
                &message,
                failure_details::command::Code::NotInWorkspace,
            );
        }
        let workspace_path = workspace.get_workspace().unwrap();
        if let Some(parent) = workspace_path.get_parent_directory() {
            let do_not_build =
                parent.get_relative_str(crate::workspace::BlazeWorkspace::DO_NOT_BUILD_FILE_NAME);
            if do_not_build.exists() {
                let message = self.get_not_in_real_workspace_error(&do_not_build);
                event_handler.handle(HandlerEvent::error(message.clone()));
                return create_options_detailed_exit_code(
                    &message,
                    failure_details::command::Code::InOutputDirectory,
                );
            }
        }
        DetailedExitCode::success()
    }

    /// `BlazeOptionHandler.parseOptions` (the second pass of commands that analyze), with the
    /// step of its `parseOptionsInternal` that Blaze runs in the runtime: `checkCwdInWorkspace`.
    /// Returns the exit code and the flags the invocation policy applied.
    // Java: com/google/devtools/build/lib/runtime/BlazeOptionHandler.java:466 BlazeOptionHandler.parseOptions
    fn parse_options(
        &self,
        command: &Arc<dyn BlazeCommand>,
        invocation_policy: &InvocationPolicy,
        args: &[String],
        options_parser: &mut OptionsParser,
        event_handler: &mut StoredEventHandler,
    ) -> (DetailedExitCode, Vec<OptionAndRawValue>) {
        // Java: com/google/devtools/build/lib/runtime/BlazeOptionHandler.java:496 BlazeOptionHandler.parseOptionsInternal
        let early_exit_code = self.check_cwd_in_workspace(command.as_ref(), event_handler);
        if !early_exit_code.is_success() {
            options_parser.set_error();
            return (early_exit_code, Vec::new());
        }
        let edit_command = command.clone();
        let mut option_handler = BlazeOptionHandler::new(
            self.runtime.get_product_name(),
            &command.annotation().rc,
            self.runtime.get_command_map().keys().cloned(),
            self.runtime.get_all_command_options_classes(),
            options_parser,
        )
        .with_invocation_policy(
            self.runtime.get_module_invocation_policy(),
            invocation_policy.clone(),
            // Allow the command to edit the options.
            Box::new(move |parser: &mut OptionsParser| edit_command.edit_options(parser)),
        );
        let result = option_handler.parse_options(args, event_handler);
        let invocation_policy_flags = option_handler.get_invocation_policy_flags().to_vec();
        (
            options_parse_result_exit_code(result),
            invocation_policy_flags,
        )
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:1063 BlazeCommandDispatcher.setShutdownReason
    fn set_shutdown_reason(&self, shutdown_reason: String) {
        set_shutdown_reason(&self.shutdown_reason, shutdown_reason);
    }
}

/// `this.shutdownReason.compareAndSet(null, shutdownReason)`.
// Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:1063 BlazeCommandDispatcher.setShutdownReason
fn set_shutdown_reason(reason: &Mutex<Option<String>>, shutdown_reason: String) {
    let mut reason = reason.lock();
    if reason.is_none() {
        *reason = Some(shutdown_reason);
    }
}

/// Releases the build arena (`blaze_util::build_arena::release_if_unused`) once a command has
/// completely ended: `exec_exclusively` has returned, so `afterCommand` ran and the command's
/// environment and its Skyframe executors (whose leases keep the arena) are gone, with the
/// threads they own and join (the evaluator's scoped workers, the consumers of spawned
/// processes, the profile writer, the BEP writer, the remote background uploads joined in
/// `close()`); the command lock is still held, so no other command has started to fill the
/// arena.
///
/// Threads of the command that are not joined may still run: the persistent worker threads
/// (`AsyncFinish-Worker-N`, the multiplexer's sender and receiver, the
/// `worker-lifecycle-manager`, the worker pipe reader), the remote cache download helpers, the
/// process reaper, the `idle-server-tasks` and `pid-file-watcher` threads of the server. Each
/// captures plain data only (worker handles, requests, paths, digests), never a frozen value, an
/// action or a spawn: rule 1 of `blaze_util::build_arena`. That is what makes the release safe
/// for them; the unowned frozen handles check their arena generation at every use
/// (`ArenaStamp`), so a breach of the rule fails loudly.
///
/// PORT: Blaze only (decision D7; docs/research/08-server-and-incrementality.md 3.3 a). Java's
/// collector frees the frozen Starlark heaps, the memo tables keyed by their values and the
/// evaluator's graph once nothing references them; Blaze's server discards its graph after
/// every command (decision D2), so everything the arena holds belongs to the command that ended.
/// Not after a crash or a `shutdown` (the server exits; a crashed command's threads may still
/// run). If a lease is still held (an executor outlived its command) nothing is released and
/// the next command shares this arena, as in a process that never releases it; the server log
/// says so (review 2026-11-26 O2). Then the allocator returns the freed memory to the system
/// (`malloc_trim`, allowed by ruling D15; it changes no output).
///
/// With a graph kept for the next command (`BLAZE_INCREMENTAL`, package I3) the graph's values
/// point into the arena and its lease keeps it: nothing is released, as Java's collector frees
/// nothing the graph references. The heaps of values the graph no longer references stay
/// until the graph is dropped (open item of package I3).
fn release_build_arena(kept_graph: bool) {
    if kept_graph && blaze_util::build_arena::active_leases() == 1 {
        crate::server_log::log(
            blaze_options::converters::LogLevel::INFO,
            "com.google.devtools.build.lib.runtime.BlazeCommandDispatcher",
            "releaseBuildArena",
            &format!(
                "build arena kept with the Skyframe graph kept for the next command; held: {}",
                blaze_util::build_arena::slot_counts()
                    .iter()
                    .map(|(name, n)| format!("{}={n}", name.rsplit("::").next().unwrap_or(name)))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
        );
    } else if !blaze_util::build_arena::release_if_unused() {
        crate::server_log::log(
            blaze_options::converters::LogLevel::WARNING,
            "com.google.devtools.build.lib.runtime.BlazeCommandDispatcher",
            "releaseBuildArena",
            &format!(
                "build arena not released after the command: {} lease(s) still held",
                blaze_util::build_arena::active_leases()
            ),
        );
    }
    crate::process_memory::release_free_memory();
}

/// The `systemOutErrPatcher.close()` of execExclusively's `finally`: run when the command's
/// `exec_exclusively` returns (after `afterCommand` and the flush of its streams) or unwinds.
struct SystemPatcherClose(blaze_events::io::SystemPatcher);

impl Drop for SystemPatcherClose {
    // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:897 BlazeCommandDispatcher.execExclusively
    fn drop(&mut self) {
        self.0.close();
    }
}

// Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:959 BlazeCommandDispatcher.bufferOut
fn buffer_out(out_err: &OutErr) -> OutErr {
    let wrapped_out: Arc<dyn blaze_events::OutputStream> = Arc::new(
        blaze_events::io::BufferedOutputStream::new(out_err.get_output_stream().clone()),
    );
    OutErr::create(wrapped_out, out_err.get_error_stream().clone())
}

// Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:964 BlazeCommandDispatcher.bufferErr
fn buffer_err(out_err: &OutErr) -> OutErr {
    let wrapped_err: Arc<dyn blaze_events::OutputStream> = Arc::new(
        blaze_events::io::BufferedOutputStream::new(out_err.get_error_stream().clone()),
    );
    OutErr::create(out_err.get_output_stream().clone(), wrapped_err)
}

// Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:979 BlazeCommandDispatcher.tee
fn tee(out_err: OutErr, additional_out_errs: Vec<OutErr>) -> OutErr {
    if additional_out_errs.is_empty() {
        return out_err;
    }
    let result = blaze_events::io::DelegatingOutErr::new();
    result.add_sink(&out_err);
    for additional_out_err in &additional_out_errs {
        result.add_sink(additional_out_err);
    }
    result.as_out_err()
}

/// The `finally` of `exec`: releases the command lock and wakes a waiting command.
struct CommandLockRelease<'a>(&'a Monitor<Option<String>>);

impl Drop for CommandLockRelease<'_> {
    fn drop(&mut self) {
        let mut current_client_description = self.0.lock();
        *current_client_description = None;
        // commandLock.notify()
        self.0.notify_all();
    }
}

/// The dispatcher the gRPC server calls (`GrpcServerImpl.executeCommand`).
// Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:192 BlazeCommandDispatcher.exec
impl CommandDispatcher for BlazeCommandDispatcher {
    fn exec(
        &self,
        invocation_policy: InvocationPolicy,
        args: Vec<String>,
        out_err: OutErr,
        locking_mode: LockingMode,
        ui_verbosity: UiVerbosity,
        client_description: String,
        first_contact_time_millis: i64,
        startup_options_tagged_with_bazel_rc: Option<Vec<(String, String)>>,
        idle_task_results_supplier: IdleTaskResultsSupplier,
        _command_extensions: Vec<Any>,
        _command_extension_reporter: Arc<dyn CommandExtensionReporter>,
    ) -> Result<BlazeCommandResult, InterruptedException> {
        self.exec_internal(
            invocation_policy,
            args,
            out_err,
            locking_mode,
            ui_verbosity,
            &client_description,
            first_contact_time_millis,
            startup_options_tagged_with_bazel_rc,
            &*idle_task_results_supplier,
        )
    }
}

// Java: com/google/devtools/build/lib/analysis/config/CoreOptionConverters.java:67 CoreOptionConverters.BLAZE_ALIASING_FLAG
const BLAZE_ALIASING_FLAG: &str = "flag_alias";

/// The state tracker `UiEventHandler.<init>` creates: `cursorControl ? new ...(clock,
/// getProgressTargetWidth()) : new ...(clock)`, then `setProgressSampleSize(uiActionsShown)` and
/// `setNewStatsSummary(newStatsSummary)` (`--experimental_stats_summary`).
// Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:241 UiEventHandler.UiEventHandler
pub(crate) fn new_ui_state_tracker(
    handler: &UiEventHandler,
    options: &UiOptions,
    skymeld_mode: bool,
    new_stats_summary: bool,
) -> blaze_buildtool::runtime::ui_state_tracker::UiStateTracker {
    use blaze_buildtool::runtime::ui_state_tracker::UiStateTracker;
    let clock: Arc<dyn blaze_events::Clock> = Arc::new(blaze_events::ui::JavaClock);
    let target_width = if handler.cursor_control() {
        handler.get_progress_target_width()
    } else {
        0
    };
    let tracker = if skymeld_mode {
        UiStateTracker::new_skymeld(clock, target_width)
    } else {
        UiStateTracker::new(clock, target_width)
    };
    tracker.set_progress_sample_size(options.ui_actions_shown);
    tracker.set_new_stats_summary(new_stats_summary);
    tracker
}

/// The `UiEventHandler` settings read from `UiOptions`.
fn ui_event_handler_options(options: &UiOptions) -> UiEventHandlerOptions {
    let mut filtered = EventKindSet::EMPTY;
    for kind in options.get_filtered_event_kinds() {
        // PORT: blaze_optdefs has its own EventKind enum; convert by name.
        let kind = EventKind::value_of(blaze_options::JavaEnum::name(&kind)).expect("EventKind");
        filtered = filtered.with(kind);
    }
    UiEventHandlerOptions {
        show_progress: options.show_progress,
        use_cursor_control: options.use_cursor_control(),
        show_timestamp: options.show_timestamp,
        experimental_ui_debug_all_events: options.experimental_ui_debug_all_events,
        attempt_to_print_relative_paths: options.attempt_to_print_relative_paths,
        filtered_event_kinds: filtered,
        max_stdout_err_bytes: options.max_stdout_err_bytes,
        show_progress_rate_limit: options.show_progress_rate_limit,
        terminal_columns: options.terminal_columns,
        progress_in_term_title: options.progress_in_term_title,
        ui_actions_shown: options.ui_actions_shown,
    }
}

/// The `DetailedExitCode` of a `BlazeOptionHandler` parse (Java:
/// `processOptionsParsingException` with code `OPTIONS_PARSE_FAILURE`).
fn options_parse_result_exit_code(result: Result<(), OptionsParseFailure>) -> DetailedExitCode {
    match result {
        Ok(()) => DetailedExitCode::success(),
        Err(failure) => DetailedExitCode::of(FailureDetail {
            message: failure.message,
            category: Some(failure_details::failure_detail::Category::Command(
                failure_details::Command {
                    code: failure_details::command::Code::OptionsParseFailure as i32,
                },
            )),
        }),
    }
}

/// The value of a boolean option: the parsed value if it was set, else its default; `false` if
/// the command has no such option.
// Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:672 BlazeCommandDispatcher.execExclusively
fn boolean_option_value(options_parser: &OptionsParser, name: &str) -> bool {
    let Some(option) = options_parser.get_option_description(name) else {
        return false;
    };
    let option_name = option.get_option_definition().get_option_name();
    match options_parser.get_option_value_description(option_name) {
        Some(cmd_line_value) => cmd_line_value
            .get_value()
            .and_then(|v| v.downcast_ref::<bool>().copied())
            .expect("ClassCastException: Boolean option"),
        None => option
            .get_option_definition()
            .get_default_value(None)
            .and_then(|v| v.downcast_ref::<bool>().copied())
            .expect("ClassCastException: Boolean option default"),
    }
}

/// Parses the Starlark options of a command that analyzes: loads the build settings the
/// skipped (`--//pkg:flag`-style) arguments name and stores their values and scopes on
/// `options_parser`.
///
/// TODO(bazel-team): When we move CoreOptions options to be defined in starlark, make sure
/// they're not passed in here during `getOptionsResult`.
// Java: com/google/devtools/build/lib/runtime/BlazeOptionHandler.java:417 BlazeOptionHandler.parseStarlarkOptions
fn parse_starlark_options(
    env: &CommandEnvironment,
    skyframe_executor: &blaze_sky_top::skyframe_executor::SkyframeExecutor,
    stored_events: &Arc<MainRepoMappingEventHandler>,
    options_parser: &mut OptionsParser,
    args: &[String],
) -> DetailedExitCode {
    // Java: new SkyframeExecutorTargetLoader(env) (reporter: NonPostingEventHandler over the
    // command's reporter; see the PORT note at the call).
    let build_setting_loader = SkyframeExecutorTargetLoader::new(
        skyframe_executor,
        env.get_relative_working_directory().clone(),
        stored_events.clone(),
    );
    let result = {
        let mut starlark_options_parser = StarlarkOptionsParser::builder()
            .build_setting_loader(&build_setting_loader)
            .native_options_parser(options_parser)
            .build();
        starlark_options_parser.parse()
    };
    match result {
        Ok(parsed) => {
            // Preconditions.checkState(starlarkOptionsParser.parse())
            blaze_util::check_state!(parsed);
            DetailedExitCode::success()
        }
        Err(StarlarkOptionsParsingError::OptionsParsing(e)) => {
            let mut e = e;
            let log_message = "Error parsing Starlark options";
            if e.get_invalid_argument().is_some() {
                for i in 0..args.len().saturating_sub(1) {
                    if Some(args[i].as_str()) == e.get_invalid_argument()
                        && !args[i + 1].starts_with('-')
                    {
                        e = OptionsParsingException::with_argument(
                            format!(
                                "{}. Did you mean {}={}?",
                                e.get_message(),
                                args[i],
                                args[i + 1]
                            ),
                            e.get_invalid_argument().expect("checked above").to_string(),
                        );
                    }
                }
            }
            // PORT: the events of loading the build settings were reported while loading (see
            // the call); only the stored posts are replayed.
            stored_events.replay_on(env.get_reporter().as_ref());
            process_options_parsing_exception(
                env.get_reporter().as_ref(),
                &e,
                log_message,
                failure_details::command::Code::StarlarkOptionsParseFailure,
            )
        }
        Err(StarlarkOptionsParsingError::Interrupted) => {
            let message = "Interrupted while parsing Starlark options";
            stored_events.replay_on(env.get_reporter().as_ref());
            env.get_reporter().handle(&Event::error(message));
            interrupted_detailed_exit_code(message)
        }
    }
}

// Java: com/google/devtools/build/lib/runtime/BlazeOptionHandler.java:623 BlazeOptionHandler.processOptionsParsingException
fn process_options_parsing_exception(
    event_handler: &dyn EventHandler,
    e: &OptionsParsingException,
    log_message: &str,
    failure_code: failure_details::command::Code,
) -> DetailedExitCode {
    // Differentiates errors stemming from an invalid argument and errors from different parts of
    // the codebase.
    let error = match e.get_invalid_argument() {
        Some(invalid_argument) => Event::error(format!(
            "{}{}{}",
            invalid_argument,
            ERROR_SEPARATOR,
            e.get_message()
        ))
        .with_tag(Some(BAD_OPTION_TAG)),
        None => Event::error(e.get_message()),
    };
    event_handler.handle(&error);
    create_options_detailed_exit_code(
        &format!("{}: {}", log_message, e.get_message()),
        failure_code,
    )
}

/// `env.getEventBus().post(new NoBuildEvent(commandName, firstContactTime, false, true,
/// env.getCommandId().toString()))` of a command that stops before it runs: the Build Event
/// Protocol of such a command consists of the events posted so far and this one.
// Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:516 BlazeCommandDispatcher.execExclusively
fn post_no_build_event(env: &CommandEnvironment) {
    // PORT: the only subscriber with an effect is the BEP's streamer (the UI's noBuild only
    // stops its progress bar, which has not started).
    if let Some(bep) = env.get_bep() {
        bep.post_no_build_event(
            Some(env.get_command_name().to_string()),
            Some(env.get_command_start_time()),
            false,
            true,
            Some(env.get_command_id().to_string()),
        );
    }
}

/// A [`DetailedExitCode`] with `ExitCode.INTERRUPTED`, `Interrupted.Code.INTERRUPTED` and the
/// provided detail message.
// Java: com/google/devtools/build/lib/util/InterruptedFailureDetails.java:30 InterruptedFailureDetails.detailedExitCode
pub(crate) fn interrupted_detailed_exit_code(message: &str) -> DetailedExitCode {
    DetailedExitCode::of(FailureDetail {
        message: message.to_string(),
        category: Some(failure_details::failure_detail::Category::Interrupted(
            failure_details::Interrupted {
                code: failure_details::interrupted::Code::Interrupted as i32,
            },
        )),
    })
}

/// An `AbruptExitException` with the [`DetailedExitCode`] of [`interrupted_detailed_exit_code`].
///
/// PORT: the overload with a `cause` keeps no cause (Blaze's `AbruptExitException` carries the
/// exit code only).
// Java: com/google/devtools/build/lib/util/InterruptedFailureDetails.java:42 InterruptedFailureDetails.abruptExitException
// Java: com/google/devtools/build/lib/util/InterruptedFailureDetails.java:55 InterruptedFailureDetails.abruptExitException
pub(crate) fn interrupted_abrupt_exit_exception(
    message: &str,
) -> crate::module::AbruptExitException {
    crate::module::AbruptExitException::new(interrupted_detailed_exit_code(message))
}

/// Converts an event recorded by `blaze_options`' rc handler into a `blaze_events` event.
fn to_event(e: &HandlerEvent) -> Event {
    let event = match e.kind {
        HandlerEventKind::Error => Event::error(e.message.clone()),
        HandlerEventKind::Warning => Event::warn(e.message.clone()),
        HandlerEventKind::Info => Event::info(e.message.clone()),
    };
    event.with_tag(e.tag.as_deref())
}

// Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:515 BlazeCommandDispatcher.replayEarlyExitEvents
fn replay_early_exit_events(
    out_err: &OutErr,
    rcfile_notes: &[String],
    stored_event_handler: &StoredEventHandler,
    module_events: &blaze_events::StoredEventHandler,
) {
    let printing_event_handler = blaze_events::printing::PrintingEventHandler::with_out_err(
        out_err.clone(),
        EventKind::ALL_EVENTS,
    );

    let bad_option = retrieve_bad_option(stored_event_handler.get_events());

    for note in rcfile_notes {
        if let Some(bad) = &bad_option
            && note.contains(bad.as_str())
        {
            printing_event_handler.handle(&Event::info(note.clone()));
        }
    }
    // PORT: Java has one StoredEventHandler for option parsing and module beforeCommand; Blaze
    // records them in two (the option handler's own type), replayed in the same order.
    for event in stored_event_handler.get_events() {
        printing_event_handler.handle(&to_event(event));
    }
    for event in module_events.get_events() {
        printing_event_handler.handle(&event);
    }
}

// Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:932 BlazeCommandDispatcher.retrieveBadOption
fn retrieve_bad_option(events: &[HandlerEvent]) -> Option<String> {
    events
        .iter()
        .filter(|e| e.tag.as_deref() == Some(BAD_OPTION_TAG))
        .map(|e| &e.message)
        .filter(|message| message.contains(ERROR_SEPARATOR))
        .map(|message| message[..message.find(ERROR_SEPARATOR).unwrap()].to_string())
        .next()
}

// Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:481 BlazeCommandDispatcher.createDetailedCommandResult
fn create_detailed_command_result(
    message: &str,
    detailed_code: failure_details::command::Code,
) -> BlazeCommandResult {
    BlazeCommandResult::detailed_exit_code(create_options_detailed_exit_code(
        message,
        detailed_code,
    ))
}

// Java: com/google/devtools/build/lib/runtime/BlazeOptionHandler.java:744 BlazeOptionHandler.createDetailedExitCode
fn create_options_detailed_exit_code(
    message: &str,
    detailed_code: failure_details::command::Code,
) -> DetailedExitCode {
    DetailedExitCode::of(FailureDetail {
        message: message.to_string(),
        category: Some(failure_details::failure_detail::Category::Command(
            failure_details::Command {
                code: detailed_code as i32,
            },
        )),
    })
}

/// The event handler of the dispatcher's computation of the main repository mapping and of
/// loading the build settings of Starlark options.
///
/// PORT: Java computes the main repository mapping on the command's Skyframe executor
/// (`env.getSkyframeExecutor().getMainRepoMapping(reporter)`), so its events reach the reporter
/// at once and the command's later evaluations find those nodes done and report nothing for
/// them. Blaze's commands create their own executor, which evaluates the same nodes again. This
/// handler reports each event at once, as Java does, and records it in the [`ReportedOnceFilter`]
/// in front of the UI handler, which drops it once when the command's evaluation reports it
/// again. Posts are stored and posted only if the command stops before `exec` (otherwise the
/// command's executor posts them).
pub(crate) struct MainRepoMappingEventHandler {
    reporter: Arc<blaze_events::Reporter>,
    filter: Arc<ReportedOnceFilter>,
    posts: blaze_events::StoredEventHandler,
}

impl MainRepoMappingEventHandler {
    fn new(reporter: Arc<blaze_events::Reporter>, filter: Arc<ReportedOnceFilter>) -> Self {
        MainRepoMappingEventHandler {
            reporter,
            filter,
            posts: blaze_events::StoredEventHandler::new(),
        }
    }

    /// Posts the stored posts on `event_handler` (the events were reported already).
    fn replay_on(&self, event_handler: &dyn blaze_events::ExtendedEventHandler) {
        self.posts.replay_on(event_handler);
    }
}

impl EventHandler for MainRepoMappingEventHandler {
    fn handle(&self, event: &Event) {
        self.filter.record(event);
        self.reporter.handle(event);
    }

    fn as_extended_event_handler(&self) -> Option<&dyn blaze_events::ExtendedEventHandler> {
        Some(self)
    }
}

impl blaze_events::ExtendedEventHandler for MainRepoMappingEventHandler {
    fn post(&self, obj: Arc<dyn blaze_events::Postable>) {
        blaze_events::ExtendedEventHandler::post(&self.posts, obj);
    }
}

/// PORT: stands in front of the UI handler and, once the dispatcher has started the command,
/// drops each event that [`MainRepoMappingEventHandler`] reported, once (see there).
pub(crate) struct ReportedOnceFilter {
    delegate: Arc<dyn EventHandler>,
    reported: Mutex<Vec<Event>>,
    dropping: std::sync::atomic::AtomicBool,
}

impl ReportedOnceFilter {
    fn new(delegate: Arc<dyn EventHandler>) -> Self {
        ReportedOnceFilter {
            delegate,
            reported: Mutex::new(Vec::new()),
            dropping: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn record(&self, event: &Event) {
        if !self.dropping.load(std::sync::atomic::Ordering::SeqCst) {
            self.reported.lock().push(event.clone());
        }
    }

    /// Forgets the recorded events: nothing will be dropped.
    fn forget_reported(&self) {
        self.reported.lock().clear();
    }

    fn start_dropping(&self) {
        self.dropping
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

impl EventHandler for ReportedOnceFilter {
    fn handle(&self, event: &Event) {
        if self.dropping.load(std::sync::atomic::Ordering::SeqCst) {
            let mut reported = self.reported.lock();
            if let Some(i) = reported.iter().position(|e| e == event) {
                reported.remove(i);
                return;
            }
        }
        self.delegate.handle(event);
    }

    fn as_extended_event_handler(&self) -> Option<&dyn blaze_events::ExtendedEventHandler> {
        self.delegate.as_extended_event_handler()
    }
}

/// The environment variable of [`inject_panic_on_detached_thread_if_requested`].
pub const INJECT_PANIC_ENV: &str = "BLAZE_DEBUG_INJECT_PANIC";

/// A test hook for the default uncaught exception handler (`BlazeRuntime
/// .setupUncaughtHandlerAtStartup`): when the server process's environment has
/// `BLAZE_DEBUG_INJECT_PANIC=<command name>`, a command of that name starts a detached thread
/// (wrapped as the server's other detached threads are, `run_catching`) that panics, so the
/// handler's crash path (exit code file, `blazeShutdownOnCrash`, halt with 37, the client's
/// "Server terminated abruptly") can be checked end to end.
///
/// PORT: Blaze-only, no Java counterpart. Read only in debug builds (`debug_assertions`); a
/// release build ignores the variable.
fn inject_panic_on_detached_thread_if_requested(command_name: &str) {
    #[cfg(debug_assertions)]
    {
        let requested = std::env::var_os(INJECT_PANIC_ENV)
            .is_some_and(|v| v.as_encoded_bytes() == command_name.as_bytes());
        if requested {
            let _ = std::thread::Builder::new()
                .name("injected-panic".to_string())
                .spawn(|| {
                    blaze_util::uncaught_exception::run_catching(|| {
                        panic!("injected panic ({INJECT_PANIC_ENV})");
                    });
                });
        }
    }
    #[cfg(not(debug_assertions))]
    let _ = command_name;
}
