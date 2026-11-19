//! `BlazeCommandDispatcher`: dispatches one command to its `BlazeCommand`, after parsing its
//! options (rc files, `--config`) and setting up the event handler.

use std::sync::Arc;

use blaze_events::ui::{UiEventHandler, UiEventHandlerOptions};
use blaze_events::{Event, EventHandler, EventKind, EventKindSet, OutErr};
use blaze_optdefs::command::common_command_options::CommonCommandOptions;
use blaze_optdefs::command::ui_options::UiOptions;
use blaze_options::rc::handler::{
    BAD_OPTION_TAG, BlazeOptionHandler, ERROR_SEPARATOR, EventKind as HandlerEventKind,
    HandlerEvent, OptionsParseFailure, StoredEventHandler,
};
use blaze_options::{OptionsParser, OptionsParsingException, PriorityCategory};
use blaze_protos::failure_details::{self, FailureDetail};
use blaze_sky_loading::package_manager::RepositoryMappingResolutionOrInterrupted;
use parking_lot::Mutex;

use crate::command::{BlazeCommand, BlazeCommandResult};
use crate::command_environment::CommandEnvironment;
use crate::exit_code::{DetailedExitCode, ExitCode};
use crate::runtime::BlazeRuntime;
use crate::starlark_options_parser::{
    SkyframeExecutorTargetLoader, StarlarkOptionsParser, StarlarkOptionsParsingError,
};

/// What to do if the command lock is not available.
// Java: com/google/devtools/build/lib/runtime/CommandDispatcher.java:35 LockingMode
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockingMode {
    WAIT,
    ERROR_OUT,
}

/// How much output to emit on the console.
// Java: com/google/devtools/build/lib/runtime/CommandDispatcher.java:41 UiVerbosity
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiVerbosity {
    QUIET,
    NORMAL,
}

/// Dispatches to the Blaze commands; that is, given a command line, this abstraction looks up
/// the appropriate command object, parses the options required by the object, and calls its
/// exec method.
// Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:106 BlazeCommandDispatcher
pub struct BlazeCommandDispatcher {
    runtime: Arc<BlazeRuntime>,
    #[allow(dead_code)]
    server_pid: i32,
    current_client_description: Mutex<Option<String>>,
    shutdown_reason: Mutex<Option<String>>,
    /// Set by [`Self::exec_exclusively`] when the command ended with `REMOTE_CACHE_EVICTED` and
    /// may be retried: Java's `RemoteCacheTransientErrorException`.
    remote_cache_transient_error: Mutex<bool>,
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
            current_client_description: Mutex::new(None),
            shutdown_reason: Mutex::new(None),
            remote_cache_transient_error: Mutex::new(false),
        }
    }

    /// Executes a single command.
    ///
    /// PORT: a non-empty invocation policy fails loudly where the runtime receives it
    /// (`BlazeRuntime::batch_main`, `server_main_single_command`). PORT: skipped, incremental:
    /// the idle tasks run between the commands of a server. Command extensions belong to the
    /// gRPC request, which the in-process server does not have. The terminal size monitor only
    /// serves the cursor-control UI. `firstContactTimeMillis` is the time the environment is
    /// created (see `CommandEnvironment::get_command_start_time`).
    // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:192 BlazeCommandDispatcher.exec
    pub fn exec(
        &self,
        args: Vec<String>,
        out_err: OutErr,
        locking_mode: LockingMode,
        ui_verbosity: UiVerbosity,
        client_description: &str,
        startup_options_tagged_with_bazel_rc: Option<Vec<(String, String)>>,
    ) -> BlazeCommandResult {
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
            return create_detailed_command_result(
                &format!("Command '{}' not found.", command_name),
                failure_details::command::Code::CommandNotFound,
            );
        };

        // Take the exclusive server lock.
        // PORT: Blaze runs one command per (in-process) server, so this lock is always free here.
        // What excludes a second Blaze process on the same output base is the client: it holds
        // the exclusive output base lock (and the shared install base lock) for the whole
        // in-process command (blaze_client `BlazeServer::communicate`), so a second client waits
        // or exits in `AcquireLock` with Bazel's `--batch` messages, never with the server's
        // "Another command (...) is running" below.
        {
            let current = self.current_client_description.lock();
            if let Some(current) = current.as_ref() {
                match locking_mode {
                    LockingMode::WAIT => {
                        blaze_util::unported!(
                            "BlazeCommandDispatcher.exec",
                            "waiting for the command lock"
                        )
                    }
                    LockingMode::ERROR_OUT => {
                        let message = format!(
                            "Another command ({}) is running. Exiting immediately.",
                            current
                        );
                        out_err.print_err_ln(&message);
                        return create_detailed_command_result(
                            &message,
                            failure_details::command::Code::AnotherCommandRunning,
                        );
                    }
                }
            }
        }
        *self.current_client_description.lock() = Some(client_description.to_string());
        let wait_time_in_ms = 0;

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
                let result = self.exec_exclusively(
                    &args,
                    out_err.clone(),
                    ui_verbosity == UiVerbosity::QUIET,
                    &command_name,
                    command.clone(),
                    wait_time_in_ms,
                    startup_options_tagged_with_bazel_rc.clone(),
                    attempt_number,
                    attempted_commands,
                );
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
            }
            result
        })();
        *self.current_client_description.lock() = None;
        result
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:363 BlazeCommandDispatcher.execExclusively
    fn exec_exclusively(
        &self,
        args: &[String],
        out_err: OutErr,
        quiet: bool,
        _command_name: &str,
        command: Arc<dyn BlazeCommand>,
        _wait_time_in_ms: i64,
        startup_options_tagged_with_bazel_rc: Option<Vec<(String, String)>>,
        attempt_number: i32,
        attempted_commands: i32,
    ) -> BlazeCommandResult {
        let command_annotation = command.annotation();
        let workspace = self.runtime.get_workspace().clone();

        let mut stored_event_handler = StoredEventHandler::default();
        let mut options_parser = self.create_options_parser(command.as_ref());

        // BlazeOptionHandler.parseOptionsAndGetConfigDefinitions, preceded by its
        // checkCwdInWorkspace (Java does both inside parseOptionsInternal).
        let all_options_classes = self.runtime.get_all_command_options_classes();
        let valid_commands: Vec<String> = self.runtime.get_command_map().keys().cloned().collect();
        let mut early_exit_code: Option<DetailedExitCode> =
            Some(self.check_cwd_in_workspace(command.as_ref(), &mut stored_event_handler));
        let rcfile_notes: Vec<String>;
        {
            let mut option_handler = BlazeOptionHandler::new(
                self.runtime.get_product_name(),
                &command_annotation.rc,
                valid_commands,
                all_options_classes,
                &mut options_parser,
            );
            if early_exit_code.as_ref().unwrap().is_success() {
                let (result, _config_definitions) = option_handler
                    .parse_options_and_get_config_definitions(args, &mut stored_event_handler);
                early_exit_code = Some(options_parse_result_exit_code(result));
            }
            rcfile_notes = option_handler.get_rcfile_notes().to_vec();
        }
        // Allow the command to edit the options (Java: inside parseOptionsInternal, after
        // parseArgsAndConfigs succeeded).
        if early_exit_code.as_ref().unwrap().is_success() {
            command.edit_options(&mut options_parser);
            // PORT: InvocationPolicyEnforcer.enforce (no invocation policy is ported).
        }
        let options = Arc::new(options_parser);

        let mut command_env_warnings = Vec::new();
        let mut env = CommandEnvironment::new(
            self.runtime.clone(),
            workspace.clone(),
            command_annotation,
            options.clone(),
            &mut command_env_warnings,
        );
        env.set_client_command_line(args.to_vec(), startup_options_tagged_with_bazel_rc.clone());

        // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:419 BlazeCommandDispatcher.execExclusively
        if attempt_number > 1 {
            out_err.print_err_ln("Found transient remote cache error, retrying the build...");
        }

        let common_options = options
            .get_options::<CommonCommandOptions>()
            .expect("CommonCommandOptions");
        // PORT: skipped, profiling: the profiler (--profile, --generate_json_trace_profile) and
        // the Starlark CPU profiler (--starlark_cpu_profile) are not started.

        let reporter = env.get_reporter().clone();
        // Both the call to env.decideKeepIncrementalState() and module.beforeCommand() may emit
        // events, but the reporter isn't setup yet. Use a stored event handler to catch those
        // events.
        let module_events = Arc::new(blaze_events::StoredEventHandler::new());
        reporter.add_handler(module_events.clone());
        // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:490 BlazeCommandDispatcher.execExclusively
        env.decide_keep_incremental_state();
        if let Some(e) = env.take_pending_exception() {
            // BlazeModuleEnvironment.exit before beforeCommand: Java reports it from
            // CommandEnvironment.beforeCommand (throwPendingException).
            env.exit(e);
        }
        for module in self.runtime.get_blaze_modules().to_vec() {
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

        // Setup stdout / stderr: outErr = tee(outErr, env.getOutputListeners()).
        // PORT: the only output listener of Bazel's modules that is ported here is
        // CommandLogModule's, and of it the part that runs with the default options: unlinking
        // the command log of a previous build. Writing the log (--write_command_log) fails
        // loudly in crate::unported_options; BuildEventServiceModule's listener (stdout/stderr
        // in the BEP's progress events) is with the BEP (crate::commands::bep).
        // Java: com/google/devtools/build/lib/runtime/CommandLogModule.java:52 CommandLogModule.getOutputListener
        if !common_options.redirect_local_instrumentation_output_writes {
            // When instrumentation output are locally written, we need to unlink old local
            // command log from previous build, if present.
            let command_log = env.get_output_base().get_relative_str("command.log");
            if let Err(io_exception) = command_log.delete() {
                env.get_reporter().handle(&Event::warn(format!(
                    "Unable to delete command log: {}",
                    io_exception
                )));
            }
        }

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
            return self.after_command(&mut env, result, &out_err);
        }

        // Setup the event handler.
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
        // PORT: bufferOut/bufferErr (BufferedOutputStream) only change write granularity.
        // PORT: DebugLoggerConfigurator.setupLogging(verbosity) sets the level of the server's
        // java.util.logging loggers; this crate writes no such log lines.
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
        reporter.add_handler(handler.clone());
        env.set_ui_event_handler(handler.clone());
        // PORT: the ANSI-allowing handler for --color=no (registerAnsiAllowingHandler) is only
        // switched to by the `run` command (Reporter.switchToAnsiAllowingHandler), whose `exec`
        // is not ported and fails loudly.

        // PORT: warnIfUsingUnusupportedEncoding: the in-process server's sun.jnu.encoding is the
        // ISO-8859-1 the client forces (see InfoItem character-encoding), which is supported.

        // Now we're ready to replay the events.
        for event in stored_event_handler.get_events() {
            reporter.handle(&to_event(event));
        }
        module_events.replay_on(reporter.as_ref());
        for warning in &command_env_warnings {
            reporter.handle(&Event::warn(warning.clone()));
        }

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

        // Notify the BlazeRuntime, so it can do some initial setup.
        // Java: CommandEnvironment.beforeCommand
        if let Err(e) = self.env_before_command(&mut env) {
            reporter.handle(&Event::error(e.get_message()));
            let result = BlazeCommandResult::detailed_exit_code(e.get_detailed_exit_code().clone());
            return self.after_command(&mut env, result, &out_err);
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
            // uses, so its events (MODULE.bazel warnings, fetches) are reported once. Blaze's
            // command creates a new executor that evaluates them again; this one stores its
            // events and reports them only if the command stops here.
            let stored_events = Arc::new(blaze_events::StoredEventHandler::new());
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
                        return self.after_command(&mut env, result, &out_err);
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
            // PORT: MainRepoMappingComputationStartingEvent only feeds the UI's progress bar
            // (cursor-control UI). PORT: skipped, profiling: the profiler task.
            // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:701 BlazeCommandDispatcher.execExclusively
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
            if !early_exit_code.is_success() {
                post_no_build_event(&env);
                let result = BlazeCommandResult::detailed_exit_code(early_exit_code);
                return self.after_command(&mut env, result, &out_err);
            }
            let mut options_parser = options_parser.expect("the options parser was rebuilt");
            // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:739 BlazeCommandDispatcher.execExclusively
            // PORT: skipped, profiling: the profiler task. A non-empty invocation policy fails
            // loudly where the runtime receives it, so there are no policy flags to re-apply.
            let mut stored_second_pass_event_handler = StoredEventHandler::default();
            let early_exit_code = self.parse_options(
                command.as_ref(),
                args,
                &mut options_parser,
                &mut stored_second_pass_event_handler,
            );
            if !early_exit_code.is_success() {
                for event in stored_second_pass_event_handler.get_events() {
                    reporter.handle(&to_event(event));
                }
                post_no_build_event(&env);
                let result = BlazeCommandResult::detailed_exit_code(early_exit_code);
                return self.after_command(&mut env, result, &out_err);
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
            let early_exit_code = parse_starlark_options(
                &env,
                &skyframe_executor,
                &stored_events,
                &mut options_parser,
                args,
            );
            drop(skyframe_executor);
            if !early_exit_code.is_success() {
                post_no_build_event(&env);
                let result = BlazeCommandResult::detailed_exit_code(early_exit_code);
                return self.after_command(&mut env, result, &out_err);
            }
            // optionHandler.getOptionsResult(). Like Java, env.getOptions() keeps the first
            // parse; the command gets this one.
            options = Arc::new(options_parser);
        }

        // PORT: the command-line BEP events (OriginalUnstructuredCommandLineEvent,
        // OriginalCommandLineEvent, CanonicalCommandLineEvent, the tool command line) are posted
        // by the commands that produce a Build Event Protocol (build and test:
        // crate::commands::bep::BepCommandSupport::before_command); for the other commands a
        // requested BEP fails loudly in crate::unported_options.

        // PORT: the options of module hooks that are not ported fail loudly here, where the
        // options are final (see crate::unported_options).
        crate::unported_options::check_unported_options(command_annotation, &options);

        // Run the command.
        let result = command.exec(&mut env, &options);
        // PORT: env.precompleteCommand posts the CommandPrecompleteEvent, whose only subscriber
        // with an effect in Bazel is the MetricsCollector (the BuildMetricsEvent); build and test
        // run it with their BEP (crate::commands::build::run_build). No module returns an exit
        // code from it.
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
    /// PORT: throws the pending module exception and runs the runtime hook. BuildMetadataEvent
    /// and GotOptionsEvent are posted with the BEP of build and test
    /// (crate::commands::bep::BepCommandSupport::before_command). No Bazel module provides an
    /// output service besides the remote module's (the remote output service fails loudly in
    /// crate::unported_options). PORT: skipped, incremental: the timestamp granularity monitor,
    /// the client environment and package path synced into the server's SkyframeExecutor
    /// (Blaze's commands create their executor from the same options).
    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:919 CommandEnvironment.beforeCommand
    fn env_before_command(
        &self,
        env: &mut CommandEnvironment,
    ) -> Result<(), crate::module::AbruptExitException> {
        if let Some(e) = env.take_pending_exception() {
            return Err(e);
        }
        self.runtime.before_command();
        Ok(())
    }

    /// `BlazeRuntime.afterCommand` plus the stream flushing of execExclusively's finally block.
    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:750 BlazeRuntime.afterCommand
    fn after_command(
        &self,
        env: &mut CommandEnvironment,
        command_result: BlazeCommandResult,
        out_err: &OutErr,
    ) -> BlazeCommandResult {
        let mut module_exit_code: Option<DetailedExitCode> = None;
        for module in self.runtime.get_blaze_modules() {
            if let Err(e) = module.after_command() {
                env.get_reporter().handle(&Event::error(e.get_message()));
                module_exit_code = DetailedExitCode::choose_more_important_with_first_if_tie(
                    module_exit_code,
                    Some(e.get_detailed_exit_code().clone()),
                );
            }
        }
        let final_command_result = match module_exit_code {
            Some(code) if !command_result.get_exit_code().is_infrastructure_failure() => {
                BlazeCommandResult::detailed_exit_code(code)
            }
            _ => command_result,
        };
        if let Some(handler) = env.get_ui_event_handler() {
            // UiEventHandler is subscribed to AfterCommandEvent / BuildComplete; it flushes its
            // buffers there.
            handler.after_command();
        }
        if let Some(reason) = env.take_shutdown_reason() {
            self.set_shutdown_reason(reason);
        }
        // Swallow IOException, as we are already in a finally clause
        let _ = out_err.get_output_stream().flush();
        let _ = out_err.get_error_stream().flush();
        final_command_result
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
                blaze_util::unported!("BlazeOptionHandler.getNotInRealWorkspaceError");
            }
        }
        DetailedExitCode::success()
    }

    /// `BlazeOptionHandler.parseOptions` (the second pass of commands that analyze), with the
    /// steps of its `parseOptionsInternal` that Blaze runs in the runtime: `checkCwdInWorkspace`
    /// and `command.editOptions`.
    /// PORT: InvocationPolicyEnforcer.enforce (no invocation policy is ported).
    // Java: com/google/devtools/build/lib/runtime/BlazeOptionHandler.java:466 BlazeOptionHandler.parseOptions
    fn parse_options(
        &self,
        command: &dyn BlazeCommand,
        args: &[String],
        options_parser: &mut OptionsParser,
        event_handler: &mut StoredEventHandler,
    ) -> DetailedExitCode {
        // Java: com/google/devtools/build/lib/runtime/BlazeOptionHandler.java:496 BlazeOptionHandler.parseOptionsInternal
        let early_exit_code = self.check_cwd_in_workspace(command, event_handler);
        if !early_exit_code.is_success() {
            options_parser.set_error();
            return early_exit_code;
        }
        let result = {
            let mut option_handler = BlazeOptionHandler::new(
                self.runtime.get_product_name(),
                &command.annotation().rc,
                self.runtime.get_command_map().keys().cloned(),
                self.runtime.get_all_command_options_classes(),
                options_parser,
            );
            option_handler.parse_options(args, event_handler)
        };
        if result.is_ok() {
            // Allow the command to edit the options.
            command.edit_options(options_parser);
        }
        options_parse_result_exit_code(result)
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:323 BlazeCommandDispatcher.setShutdownReason
    fn set_shutdown_reason(&self, shutdown_reason: String) {
        let mut reason = self.shutdown_reason.lock();
        if reason.is_none() {
            *reason = Some(shutdown_reason);
        }
    }
}

// Java: com/google/devtools/build/lib/analysis/config/CoreOptionConverters.java:67 CoreOptionConverters.BLAZE_ALIASING_FLAG
const BLAZE_ALIASING_FLAG: &str = "flag_alias";

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
    stored_events: &Arc<blaze_events::StoredEventHandler>,
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
            // PORT: the events of loading the build settings (stored, see the call) come before
            // the error, as they do when Java reports them while loading.
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

// Java: com/google/devtools/build/lib/util/InterruptedFailureDetails.java:30 InterruptedFailureDetails.detailedExitCode
/// `env.getEventBus().post(new NoBuildEvent(...))` of a command that stops before it runs: the
/// Build Event Protocol of such a command consists of the events posted so far and this one.
///
/// PORT: Blaze's BEP support starts inside `build` and `test` (crate::commands::bep), after the
/// dispatcher's early exits, so a command that requested BEP files and stops here would leave
/// none; that fails loudly. Without BEP files the event has no subscriber.
// Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:516 BlazeCommandDispatcher.execExclusively
fn post_no_build_event(env: &CommandEnvironment) {
    use blaze_optdefs::misc::build_event_stream_options::BuildEventStreamOptions;
    if let Some(o) = env.get_options().get_options::<BuildEventStreamOptions>()
        && (!o.build_event_text_file.is_empty()
            || !o.build_event_binary_file.is_empty()
            || !o.build_event_json_file.is_empty())
    {
        blaze_util::unported!(
            "NoBuildEvent",
            "the Build Event Protocol (--build_event_*_file) of a command that stops before it runs"
        );
    }
}

fn interrupted_detailed_exit_code(message: &str) -> DetailedExitCode {
    DetailedExitCode::of(FailureDetail {
        message: message.to_string(),
        category: Some(failure_details::failure_detail::Category::Interrupted(
            failure_details::Interrupted {
                code: failure_details::interrupted::Code::Interrupted as i32,
            },
        )),
    })
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
