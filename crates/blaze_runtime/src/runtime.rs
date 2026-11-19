//! `BlazeRuntime`: the Blaze server, its startup (`main`, `batchMain`, `serverMain`,
//! `newRuntime`) and its module, command and info-item registries.

use std::sync::{Arc, OnceLock};

use blaze_events::OutErr;
use blaze_optdefs::command::blaze_server_startup_options::BlazeServerStartupOptions;
use blaze_options::definition::{OptionDefinitionRef, OptionsClassRef};
use blaze_options::invocation_policy::{CommandNameCacheInstance, InvocationPolicyParser};
use blaze_options::{OptionsParser, OptionsParsingException, PriorityCategory};
use blaze_protos::blaze::invocation_policy::InvocationPolicy;
use blaze_protos::command_server::{ExecRequest, RunRequest, RunResponse};
use blaze_protos::failure_details::{self, FailureDetail, failure_detail};
use blaze_server::{GrpcServerImpl, PidFileWatcher, ShutdownHooks};
use blaze_shell::thread_interrupt::ThreadInterrupt;
use blaze_vfs::{FileSystemRef, Path, PathFragment, UnixFileSystem};
use indexmap::IndexMap;

use crate::command::{BlazeCommand, BlazeCommandResult};
use crate::command_name_cache_impl::CommandNameCacheImpl;
use crate::custom_exit_code_publisher::{CustomExitCodePublisher, CustomFailureDetailPublisher};
use crate::directories::{BlazeDirectories, ServerDirectories};
use crate::dispatcher::{BlazeCommandDispatcher, LockingMode, UiVerbosity};
use crate::exit_code::{DetailedExitCode, ExitCode};
use crate::info_item::InfoItem;
use crate::interrupt::InterruptSignalHandler;
use crate::module::{AbruptExitException, BlazeModule, ServerBuilder};
use crate::options_result::OptionsParsingResult;
use crate::workspace::BlazeWorkspace;

/// The runfiles prefix `BazelRuleClassProvider.setup` sets (`setRunfilesPrefix("_main")`), which
/// `ConfiguredRuleClassProvider.getRunfilesPrefix` returns.
/// PORT: move to blaze_rules (BazelRuleClassProvider) once it exists.
// Java: com/google/devtools/build/lib/bazel/rules/BazelRuleClassProvider.java:232 BazelRuleClassProvider.setRunfilesPrefix
pub const RUNFILES_PREFIX: &str = "_main";

/// The startup options classes of Blaze core.
// Java: com/google/devtools/build/lib/runtime/BlazeCommandUtils.java:33 BlazeCommandUtils.DEFAULT_STARTUP_OPTIONS
fn default_startup_options() -> Vec<OptionsClassRef> {
    use blaze_options::OptionsClass;
    vec![
        BlazeServerStartupOptions::class(),
        blaze_optdefs::command::host_jvm_startup_options::HostJvmStartupOptions::class(),
    ]
}

/// The runtime of a Blaze server: modules, commands, info items and the workspace.
// Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:162 BlazeRuntime
pub struct BlazeRuntime {
    file_system: FileSystemRef,
    blaze_modules: Vec<Arc<dyn BlazeModule>>,
    command_map: IndexMap<String, Arc<dyn BlazeCommand>>,
    info_items: IndexMap<String, Arc<dyn InfoItem>>,
    startup_options_provider: Arc<OptionsParsingResult>,
    product_name: String,
    workspace: OnceLock<Arc<BlazeWorkspace>>,
    /// The environment of the running command, between `beforeCommand` and `afterCommand`
    /// (PORT: the parts `fillInCrashContext` and `cleanUpForCrash` use).
    env: parking_lot::Mutex<Option<CrashEnv>>,
    /// The exit code declared by the first crash (Java `storedExitCode`).
    stored_exit_code: parking_lot::Mutex<Option<crate::exit_code::DetailedExitCode>>,
    /// `abruptShutdownHandler`: the gRPC server's `prepareForAbruptShutdown` (null in batch
    /// mode).
    abrupt_shutdown_handler: Option<Arc<dyn Fn() + Send + Sync>>,
}

/// What the crash handling uses of the command's `CommandEnvironment`.
#[derive(Clone)]
pub(crate) struct CrashEnv {
    pub(crate) reporter: Arc<blaze_events::Reporter>,
    pub(crate) event_bus: Arc<crate::command_environment::CommandEventBus>,
    /// `CommandEnvironment.shutdownReasonConsumer` (the dispatcher's `setShutdownReason`).
    /// PORT: kept here, with the command thread, because `CommandEnvironment` has no such
    /// fields yet.
    pub(crate) shutdown_reason_consumer: Arc<dyn Fn(String) + Send + Sync>,
    /// `CommandEnvironment.commandThread`: the thread that created the environment.
    pub(crate) command_thread: ThreadInterrupt,
}

impl CrashEnv {
    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:476 CommandEnvironment.notifyOnCrash
    fn notify_on_crash(&self, message: String) {
        (self.shutdown_reason_consumer)(message);
        if !ThreadInterrupt::current().same_thread(&self.command_thread) {
            // Give shutdown hooks priority in JVM and stop generating more data for modules to
            // consume.
            self.command_thread.interrupt();
        }
    }
}

impl BlazeRuntime {
    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:526 BlazeRuntime.getFileSystem
    pub fn get_file_system(&self) -> &FileSystemRef {
        &self.file_system
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:530 BlazeRuntime.getWorkspace
    pub fn get_workspace(&self) -> &Arc<BlazeWorkspace> {
        self.workspace.get().expect("workspace not initialized")
    }

    /// Returns the directory where the server keeps its state (`<output_base>/server`).
    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:539 BlazeRuntime.getServerDirectory
    pub fn get_server_directory(&self) -> Path {
        self.get_workspace()
            .get_directories()
            .get_output_base()
            .get_child("server")
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:570 BlazeRuntime.getInfoItems
    pub fn get_info_items(&self) -> &IndexMap<String, Arc<dyn InfoItem>> {
        &self.info_items
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:574 BlazeRuntime.getBlazeModules
    pub fn get_blaze_modules(&self) -> &[Arc<dyn BlazeModule>] {
        &self.blaze_modules
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:862 BlazeRuntime.getStartupOptionsProvider
    pub fn get_startup_options_provider(&self) -> &OptionsParsingResult {
        &self.startup_options_provider
    }

    /// The startup options provider, shared (for the BEP's command-line events).
    pub fn get_startup_options_provider_arc(&self) -> Arc<OptionsParsingResult> {
        self.startup_options_provider.clone()
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:866 BlazeRuntime.getCommandMap
    pub fn get_command_map(&self) -> &IndexMap<String, Arc<dyn BlazeCommand>> {
        &self.command_map
    }

    /// The invocation policy the modules added (`ServerBuilder.addInvocationPolicy`, merged).
    /// No Bazel module calls `addInvocationPolicy`, so it is the empty policy.
    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:318 BlazeRuntime.getModuleInvocationPolicy
    pub fn get_module_invocation_policy(&self) -> InvocationPolicy {
        InvocationPolicy::default()
    }

    /// Returns the product name (lowercase, e.g. "bazel").
    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:1616 BlazeRuntime.getProductName
    pub fn get_product_name(&self) -> &str {
        &self.product_name
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:302 BlazeRuntime.addCommand
    fn add_command(&mut self, command: Arc<dyn BlazeCommand>) {
        let name = command.annotation().name().to_string();
        if self.command_map.contains_key(&name) {
            panic!(
                "IllegalStateException: Command name or alias {} is already used.",
                name
            );
        }
        self.command_map.insert(name, command);
    }

    /// Hook method called by the BlazeCommandDispatcher prior to the dispatch of each command.
    /// PORT: skipped, memory instrumentation: the memory profiler (`--memory_profile`).
    /// PORT: skipped, incremental: the idle tasks of the server between commands (GC and interner
    /// shrinking, install base and action cache garbage collection).
    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:636 BlazeRuntime.beforeCommand
    pub(crate) fn before_command(
        &self,
        env: &crate::command_environment::CommandEnvironment,
        shutdown_reason_consumer: Arc<dyn Fn(String) + Send + Sync>,
    ) {
        *self.env.lock() = Some(CrashEnv {
            reporter: env.get_reporter().clone(),
            event_bus: env.get_event_bus().clone(),
            shutdown_reason_consumer,
            // The CommandEnvironment is created on the command thread, which calls this.
            command_thread: ThreadInterrupt::current(),
        });
    }

    /// `afterCommand`'s `notifyCommandComplete(finalCommandResult.getDetailedExitCode())`:
    /// declares `exit_code` as the command's; returns the exit code to use instead if another
    /// thread declared one first (a crash).
    ///
    /// PORT: the `CommandCompleteEvent` is not posted here: no ported subscriber reads it on a
    /// normal command end (the crash path posts it, see `clean_up_for_crash`).
    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:728 BlazeRuntime.notifyCommandComplete
    pub(crate) fn notify_command_complete(
        &self,
        exit_code: &crate::exit_code::DetailedExitCode,
    ) -> Option<crate::exit_code::DetailedExitCode> {
        if !self.declare_exit_code(exit_code) {
            // This command has already been called, presumably because there is a race between
            // the main thread and a worker thread that crashed. Don't try to arbitrate the
            // dispute. If the main thread won the race (unlikely, but possible), this may be
            // incorrectly logged as a success.
            return self.stored_exit_code.lock().clone();
        }
        None
    }

    /// The end of `afterCommand`: `storedExitCode.set(null)`.
    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:835 BlazeRuntime.afterCommand
    pub(crate) fn clear_stored_exit_code(&self) {
        *self.stored_exit_code.lock() = None;
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:881 BlazeRuntime.prepareForAbruptShutdown
    pub fn prepare_for_abrupt_shutdown(&self) {
        if let Some(abrupt_shutdown_handler) = &self.abrupt_shutdown_handler {
            abrupt_shutdown_handler();
        }
    }

    /// The part of `afterCommand` that ends the command for the crash handling.
    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:752 BlazeRuntime.afterCommand
    pub(crate) fn clear_env(&self) {
        *self.env.lock() = None;
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:713 BlazeRuntime.getCrashExitCode
    pub fn get_crash_exit_code(&self) -> Option<crate::exit_code::DetailedExitCode> {
        self.stored_exit_code.lock().clone()
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:717 BlazeRuntime.declareExitCode
    fn declare_exit_code(&self, detailed_exit_code: &crate::exit_code::DetailedExitCode) -> bool {
        let mut stored = self.stored_exit_code.lock();
        if stored.is_some() {
            return false;
        }
        *stored = Some(detailed_exit_code.clone());
        true
    }

    /// Calls each module's `blazeShutdownOnCrash`.
    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:888 BlazeRuntime.shutDownModulesOnCrash
    fn shut_down_modules_on_crash(&self, exit_code: &crate::exit_code::DetailedExitCode) {
        for module in &self.blaze_modules {
            module.blaze_shutdown_on_crash(exit_code);
        }
    }

    /// Performs all shutdown steps.
    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:871 BlazeRuntime.shutdown
    pub fn shutdown(&self) {
        for module in &self.blaze_modules {
            module.blaze_shutdown();
        }
    }

    /// All the options classes of all commands (deduplicated, in first-seen order): what
    /// `BlazeOptionHandler` uses as fallback data for `common` rc options.
    pub fn get_all_command_options_classes(&self) -> Vec<OptionsClassRef> {
        let mut seen = indexmap::IndexSet::new();
        for command in self.command_map.values() {
            for class in
                crate::command_registry::get_options(command.annotation(), &self.blaze_modules)
            {
                seen.insert(ClassKey(class));
            }
        }
        seen.into_iter().map(|k| k.0).collect()
    }

    /// Main method for the Blaze server startup: the process the client starts as its server
    /// (`daemonize`, or `exec` in `--batch` mode). `args` are internal strings (Java transforms
    /// them with `StringEncoding.platformToInternal` first; the `blaze` binary's argv already
    /// is).
    ///
    /// The in-process server of Blaze's own client calls [`BlazeRuntime::batch_main`] or
    /// [`BlazeRuntime::server_main_single_command`] directly instead.
    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:911 BlazeRuntime.main
    pub fn main(modules: Vec<Arc<dyn BlazeModule>>, args: Vec<String>) -> ! {
        // PORT: the dispositions the JVM installs before main (see crate::jvm_signals).
        crate::jvm_signals::install();
        Self::setup_uncaught_handler_at_startup_with_args(&args);
        // blaze.cc will put --batch first if the user set it.
        if args.first().map(String::as_str) == Some("--batch") {
            // Run Blaze in batch mode.
            let code = Self::batch_main(modules, args);
            Self::exit(code);
        }
        // PORT: skipped, logging ("Starting Bazel server with pid %d, args %s").
        let code = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // Run Blaze in server mode.
            Self::server_main(modules, OutErr::system_out_err().clone(), &args)
        })) {
            Ok(code) => code,
            // catch (RuntimeException | Error e) { // A definite bug...
            Err(payload) => {
                let crash = crate::bugreport::Crash::from_panic(payload.as_ref());
                crate::bugreport::BugReport::handle_crash(
                    &crash,
                    crate::bugreport::CrashContext::keep_alive().with_args(&args),
                );
                crash
                    .get_detailed_exit_code()
                    .get_exit_code()
                    .get_numeric_exit_code()
            }
        };
        Self::exit(code)
    }

    /// `ProcessHandle.current().children().forEach(ProcessHandle::destroyForcibly)`, then
    /// `System.exit(exitCode)`, which runs the shutdown hooks.
    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:937 BlazeRuntime.exit
    fn exit(exit_code: i32) -> ! {
        // b/177077523: Best effort to kill all child processes. If there is any child process
        // running, System.exit will block forever.
        for child in child_processes() {
            // SAFETY: plain syscall.
            unsafe {
                libc::kill(child, libc::SIGKILL);
            }
        }
        // PORT: Rust has no shutdown hooks; the registered ShutdownHooks run here, as the JVM
        // runs them in System.exit.
        blaze_server::shutdown_hooks::run_registered_shutdown_hooks();
        let _ = OutErr::system_out_err().get_error_stream().flush();
        let _ = OutErr::system_out_err().get_output_stream().flush();
        std::process::exit(exit_code)
    }

    /// The JVM's slow-interrupt message suffix the modules contribute.
    ///
    /// PORT: no Bazel module overrides `getSlowThreadInterruptMessageSuffix` at the pin, and the
    /// hook is not in the Rust `BlazeModule` trait; the result is always null.
    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:1028 BlazeRuntime.getSlowInterruptMessageSuffix
    fn get_slow_interrupt_message_suffix(_modules: &[Arc<dyn BlazeModule>]) -> Option<String> {
        None
    }

    /// A main method that does not send email. The return value indicates the desired exit
    /// status of the program.
    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:1200 BlazeRuntime.serverMain
    fn server_main(
        blaze_modules: Vec<Arc<dyn BlazeModule>>,
        out_err: OutErr,
        args: &[String],
    ) -> i32 {
        let mut sigint_handler: Option<InterruptSignalHandler> = None;
        let result = (|| -> Result<i32, NewRuntimeError> {
            let rpc_server_ref: Arc<OnceLock<Arc<GrpcServerImpl>>> = Arc::new(OnceLock::new());
            let prepare_for_abrupt_shutdown: Arc<dyn Fn() + Send + Sync> = {
                let rpc_server_ref = rpc_server_ref.clone();
                Arc::new(move || {
                    rpc_server_ref
                        .get()
                        .expect("NullPointerException: rpcServerRef")
                        .prepare_for_abrupt_shutdown()
                })
            };
            let runtime =
                Self::new_runtime(blaze_modules, args, Some(prepare_for_abrupt_shutdown))?;

            // server.pid was written in the C++ launcher after fork() but before exec(). The
            // client only accesses the pid file after connecting to the socket which ensures that
            // it gets the correct pid value.
            let pid_file = runtime.get_server_directory().get_child("server.pid.txt");
            let server_pid = read_pid_file(&pid_file).map_err(NewRuntimeError::Abrupt)?;
            let pid_file_watcher = Arc::new(PidFileWatcher::new(pid_file.clone(), server_pid));
            pid_file_watcher.start();

            let shutdown_hooks = ShutdownHooks::create_and_register();
            shutdown_hooks.cleanup_pid_file(pid_file, pid_file_watcher.clone());

            let dispatcher = Arc::new(BlazeCommandDispatcher::new(runtime.clone(), server_pid));
            let startup_options = runtime
                .startup_options_provider
                .get_options::<BlazeServerStartupOptions>()
                .expect("BlazeServerStartupOptions");
            let rpc_server = GrpcServerImpl::create(
                dispatcher.clone(),
                shutdown_hooks,
                pid_file_watcher,
                // runtime.clock
                Arc::new(blaze_events::ui::JavaClock),
                startup_options.command_port,
                runtime.get_server_directory(),
                server_pid,
                startup_options.max_idle_seconds,
                startup_options.shutdown_on_low_sys_mem,
                startup_options.idle_server_tasks,
                Self::get_slow_interrupt_message_suffix(runtime.get_blaze_modules()),
            );
            let _ = rpc_server_ref.set(rpc_server.clone());

            // Register the signal handler.
            let interrupted_server = rpc_server.clone();
            sigint_handler = Some(InterruptSignalHandler::new(move |_| {
                // PORT: skipped, logging ("User interrupt").
                interrupted_server.interrupt();
            }));

            rpc_server.serve().map_err(|e| {
                NewRuntimeError::Abrupt(AbruptExitException::new(
                    e.get_detailed_exit_code().clone(),
                ))
            })?;
            runtime.shutdown();
            dispatcher.shutdown();
            Ok(ExitCode::SUCCESS.get_numeric_exit_code())
        })();
        let code = match result {
            Ok(code) => code,
            Err(NewRuntimeError::Options(e)) => {
                out_err.print_err_ln(e.get_message());
                ExitCode::COMMAND_LINE_ERROR.get_numeric_exit_code()
            }
            Err(NewRuntimeError::Abrupt(e)) => {
                out_err.print_err_ln(e.get_message());
                // PORT: e.printStackTrace(...): a Java stack trace has no Rust counterpart.
                if let Some(failure_detail) = e.get_detailed_exit_code().get_failure_detail() {
                    CustomFailureDetailPublisher::maybe_write_failure_detail_file(failure_detail);
                }
                e.get_detailed_exit_code()
                    .get_exit_code()
                    .get_numeric_exit_code()
            }
        };
        // finally
        if let Some(sigint_handler) = sigint_handler {
            sigint_handler.uninstall();
        }
        code
    }

    /// Splits given options into two lists: arguments matching startup options and everything
    /// else, preserving order in each list. Relies on startup options always being in
    /// `--flag=ARG` form (blaze.cc's GetServerExeArgs guarantees it).
    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:990 BlazeRuntime.splitStartupOptions
    pub fn split_startup_options(
        modules: &[Arc<dyn BlazeModule>],
        args: &[String],
    ) -> (Vec<String>, Vec<String>) {
        let mut prefixes = Vec::new();
        let mut startup_options: Vec<OptionDefinitionRef> = Vec::new();
        for default_options in Self::get_startup_options(modules) {
            startup_options.extend(OptionsParser::get_option_definitions(default_options));
        }
        for option_definition in startup_options {
            prefixes.push(format!("--{}", option_definition.get_option_name()));
            if option_definition.uses_boolean_value_syntax() {
                prefixes.push(format!("--no{}", option_definition.get_option_name()));
            }
        }

        let mut startup_args = Vec::new();
        let mut other_args: Vec<String> = args.to_vec();
        let mut i = 0;
        while i < other_args.len() {
            let arg = &other_args[i];
            if !arg.starts_with("--") {
                break; // stop at command - all startup options would be specified before it.
            }
            if prefixes.iter().any(|p| arg.starts_with(p.as_str())) {
                startup_args.push(other_args.remove(i));
            } else {
                i += 1;
            }
        }
        (startup_args, other_args)
    }

    /// `BlazeCommandUtils.getStartupOptions`: the core startup options plus every module's.
    /// PORT: Java collects into a HashSet; the order of classes is not observable here.
    // Java: com/google/devtools/build/lib/runtime/BlazeCommandUtils.java:48 BlazeCommandUtils.getStartupOptions
    pub fn get_startup_options(modules: &[Arc<dyn BlazeModule>]) -> Vec<OptionsClassRef> {
        let mut options = indexmap::IndexSet::new();
        for c in default_startup_options() {
            options.insert(ClassKey(c));
        }
        for module in modules {
            for c in module.get_startup_options() {
                options.insert(ClassKey(c));
            }
        }
        options.into_iter().map(|k| k.0).collect()
    }

    /// A main method that runs blaze commands in batch mode. The return value indicates the
    /// desired exit status of the program.
    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:1085 BlazeRuntime.batchMain
    ///
    /// PORT: `BlazeRuntime.main` calls `exit(batchMain(...))` outside its try, so a crash that
    /// escapes the dispatcher reaches the default uncaught exception handler, which halts with
    /// the crash's exit code; the catch here is that handler.
    pub fn batch_main(modules: Vec<Arc<dyn BlazeModule>>, args: Vec<String>) -> i32 {
        Self::setup_uncaught_handler_at_startup_with_args(&args);
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            Self::batch_main_internal(modules, args.clone())
        })) {
            Ok(code) => code,
            Err(payload) => Self::uncaught_exception(payload.as_ref(), &args),
        }
    }

    /// Make sure async threads cannot be orphaned at startup. This method makes sure bugs are
    /// reported and the proper exit code is reported.
    ///
    /// PORT: the panic hook records panics for the crash handling (crate::bugreport); the
    /// handler of the server's threads (`blaze_server::uncaught_exception`) is Java's default
    /// uncaught exception handler: `BugReport.handleCrash(throwable, args)`, which halts.
    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:1610 BlazeRuntime.setupUncaughtHandlerAtStartup
    fn setup_uncaught_handler_at_startup() {
        Self::setup_uncaught_handler_at_startup_with_args(&[]);
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:1610 BlazeRuntime.setupUncaughtHandlerAtStartup
    fn setup_uncaught_handler_at_startup_with_args(args: &[String]) {
        crate::bugreport::install_panic_hook();
        let args = args.to_vec();
        blaze_server::uncaught_exception::set_uncaught_exception_handler(Arc::new(
            move |message: &str| {
                // Java: com/google/devtools/build/lib/bugreport/BugReport.java:244 BugReport.handleCrash
                let crash = crate::bugreport::Crash::from_message(message.to_owned());
                crate::bugreport::BugReport::handle_crash(
                    &crash,
                    crate::bugreport::CrashContext::halt().with_args(&args),
                );
                unreachable!("Should have halted")
            },
        ));
    }

    /// The default uncaught exception handler: `BugReport.handleCrash(throwable, args)`, which
    /// halts.
    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:1612 BlazeRuntime.setupUncaughtHandlerAtStartup
    fn uncaught_exception(payload: &(dyn std::any::Any + Send), args: &[String]) -> ! {
        // Java: com/google/devtools/build/lib/bugreport/BugReport.java:244 BugReport.handleCrash
        let crash = crate::bugreport::Crash::from_panic(payload);
        crate::bugreport::BugReport::handle_crash(
            &crash,
            crate::bugreport::CrashContext::halt().with_args(args),
        );
        unreachable!("Should have halted")
    }

    fn batch_main_internal(modules: Vec<Arc<dyn BlazeModule>>, args: Vec<String>) -> i32 {
        // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:1087 BlazeRuntime.batchMain
        let _signal_handler = crate::interrupt::capture_sigint();
        let (startup_args, other_args) = Self::split_startup_options(&modules, &args);

        let runtime = match Self::new_runtime(modules, &startup_args, None) {
            Ok(r) => r,
            Err(NewRuntimeError::Options(e)) => {
                OutErr::system_out_err().print_err_ln(e.get_message());
                return ExitCode::COMMAND_LINE_ERROR.get_numeric_exit_code();
            }
            Err(NewRuntimeError::Abrupt(e)) => {
                OutErr::system_out_err().print_err_ln(e.get_message());
                return e
                    .get_detailed_exit_code()
                    .get_exit_code()
                    .get_numeric_exit_code();
            }
        };
        let startup_options = runtime
            .startup_options_provider
            .get_options::<BlazeServerStartupOptions>()
            .expect("BlazeServerStartupOptions");
        // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:1102 BlazeRuntime.batchMain
        let policy = match InvocationPolicyParser::parse_policy(Some(
            startup_options.invocation_policy.as_str(),
        )) {
            Ok(policy) => policy,
            Err(e) => {
                OutErr::system_out_err().print_err_ln(e.get_message());
                return ExitCode::COMMAND_LINE_ERROR.get_numeric_exit_code();
            }
        };

        let startup_options_from_command_line: Vec<(String, String)> = startup_args
            .iter()
            .map(|option| (String::new(), option.clone()))
            .collect();

        let dispatcher = BlazeCommandDispatcher::new(
            runtime.clone(),
            BlazeCommandDispatcher::UNKNOWN_SERVER_PID,
        );
        let result = match dispatcher.exec(
            policy,
            other_args,
            OutErr::system_out_err().clone(),
            LockingMode::ERROR_OUT,
            if startup_options.quiet {
                UiVerbosity::QUIET
            } else {
                UiVerbosity::NORMAL
            },
            "batch client",
            Some(startup_options_from_command_line),
        ) {
            Ok(result) => result,
            // This is almost main(), so it's okay to just swallow it. We are exiting soon.
            Err(_interrupted) => {
                runtime.shutdown();
                dispatcher.shutdown();
                return ExitCode::INTERRUPTED.get_numeric_exit_code();
            }
        };
        let Some(request) = result.get_exec_request() else {
            // Simple case: we are given an exit code
            runtime.shutdown();
            dispatcher.shutdown();
            return result.get_exit_code().get_numeric_exit_code();
        };

        // Not so simple case: we need to execute a binary on shutdown. exec() is not accessible
        // from Java and is impossible on Windows in any case, so we just execute the binary after
        // getting out of the way as completely as possible and forward its exit code.
        // When this code is executed, no locks are held: the client lock is released by the
        // client before it executes any command and the server lock is handled by
        // BlazeCommandDispatcher, whose job is done by the time we get here.
        // PORT: in-process, the client's locks are still held (O_CLOEXEC does not apply to a
        // child process that is not exec'd from this one).
        // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:1137 BlazeRuntime.batchMain
        runtime.shutdown();
        dispatcher.shutdown();
        _signal_handler.uninstall();
        run_exec_request_in_batch_mode(request)
    }

    /// Server mode for exactly one command: `serverMain` creates the runtime and the gRPC
    /// server, and `GrpcServerImpl.run` executes the request (`executeCommand`). The in-process
    /// server then shuts down.
    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:1200 BlazeRuntime.serverMain
    pub fn server_main_single_command(
        modules: Vec<Arc<dyn BlazeModule>>,
        startup_args: Vec<String>,
        request: RunRequest,
    ) -> RunResponse {
        Self::setup_uncaught_handler_at_startup();
        let args = startup_args.clone();
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            Self::server_main_single_command_internal(modules, startup_args, request)
        })) {
            Ok(response) => response,
            // catch (RuntimeException | Error e) { // A definite bug...
            Err(payload) => {
                // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:930 BlazeRuntime.main
                // PORT: the server process exits with the crash's code; in-process it is the
                // response's, with the termination expected.
                let crash = crate::bugreport::Crash::from_panic(payload.as_ref());
                crate::bugreport::BugReport::handle_crash(
                    &crash,
                    crate::bugreport::CrashContext::keep_alive().with_args(&args),
                );
                RunResponse {
                    finished: true,
                    termination_expected: true,
                    exit_code: crash
                        .get_detailed_exit_code()
                        .get_exit_code()
                        .get_numeric_exit_code(),
                    failure_detail: crash.get_detailed_exit_code().get_failure_detail().cloned(),
                    ..Default::default()
                }
            }
        }
    }

    fn server_main_single_command_internal(
        modules: Vec<Arc<dyn BlazeModule>>,
        startup_args: Vec<String>,
        request: RunRequest,
    ) -> RunResponse {
        // PORT: a server that fails to start exits with this code; in-process it is the
        // response's.
        let exit_response = |exit_code: i32| RunResponse {
            finished: true,
            exit_code,
            ..Default::default()
        };
        let runtime = match Self::new_runtime(modules, &startup_args, None) {
            Ok(r) => r,
            Err(NewRuntimeError::Options(e)) => {
                OutErr::system_out_err().print_err_ln(e.get_message());
                return exit_response(ExitCode::COMMAND_LINE_ERROR.get_numeric_exit_code());
            }
            Err(NewRuntimeError::Abrupt(e)) => {
                OutErr::system_out_err().print_err_ln(e.get_message());
                return exit_response(
                    e.get_detailed_exit_code()
                        .get_exit_code()
                        .get_numeric_exit_code(),
                );
            }
        };
        // server.pid was written in the C++ launcher; in-process the server is this process.
        let server_pid = std::process::id() as i32;
        // The server is a daemon (ExecuteDaemon: setsid), so its children have no controlling
        // terminal and a Ctrl-C reaches only the client, which cancels the command
        // (blaze_client::signal_handler). In-process, the children get a session of their own.
        // Java: src/main/tools/daemonize.cc:270 main
        blaze_shell::java_process::set_start_children_in_new_session(true);
        let dispatcher = BlazeCommandDispatcher::new(runtime.clone(), server_pid);

        // Java: com/google/devtools/build/lib/server/GrpcServerImpl.java:539 GrpcServerImpl.executeCommand
        // PORT: the request cookie, cancellation and preemption are not ported; output streams
        // go straight to this process's stdout/stderr.
        // Java: com/google/devtools/build/lib/server/GrpcServerImpl.java:784 GrpcServerImpl.platformBytesToInternalString
        let platform_bytes_to_internal_string =
            |bytes: &[u8]| blaze_util::string_encoding::latin1_decode(bytes).into_owned();
        // Convert the startup options record to Java strings, source first.
        let startup_options_tagged_with_bazel_rc: Vec<(String, String)> = request
            .startup_options
            .iter()
            .map(|o| {
                (
                    platform_bytes_to_internal_string(&o.source),
                    platform_bytes_to_internal_string(&o.option),
                )
            })
            .collect();
        // Transform args into Bazel's internal string representation.
        let args: Vec<String> = request
            .arg
            .iter()
            .map(|a| platform_bytes_to_internal_string(a))
            .collect();
        // Java: com/google/devtools/build/lib/server/GrpcServerImpl.java:607 GrpcServerImpl.executeCommand
        let result = match InvocationPolicyParser::parse_policy(Some(&request.invocation_policy)) {
            Ok(policy) => match dispatcher.exec(
                policy,
                args,
                OutErr::system_out_err().clone(),
                if request.block_for_lock {
                    LockingMode::WAIT
                } else {
                    LockingMode::ERROR_OUT
                },
                if request.quiet {
                    UiVerbosity::QUIET
                } else {
                    UiVerbosity::NORMAL
                },
                &request.client_description,
                Some(startup_options_tagged_with_bazel_rc),
            ) {
                Ok(result) => result,
                // Java: com/google/devtools/build/lib/server/GrpcServerImpl.java:637 GrpcServerImpl.executeCommand
                // PORT: Java also clears the command id ("the client will ignore it"); the
                // in-process response has none.
                Err(_interrupted) => {
                    BlazeCommandResult::detailed_exit_code(DetailedExitCode::of(FailureDetail {
                        message: "Command dispatch interrupted".to_string(),
                        category: Some(failure_detail::Category::Interrupted(
                            failure_details::Interrupted {
                                code: failure_details::interrupted::Code::Interrupted as i32,
                            },
                        )),
                    }))
                }
            },
            Err(e) => {
                // Java: com/google/devtools/build/lib/server/GrpcServerImpl.java:623 GrpcServerImpl.executeCommand
                OutErr::system_out_err().print_err_ln(e.get_message());
                BlazeCommandResult::detailed_exit_code(DetailedExitCode::of(FailureDetail {
                    message: format!("Invocation policy parsing failed: {}", e.get_message()),
                    category: Some(failure_detail::Category::Command(
                        failure_details::Command {
                            code: failure_details::command::Code::InvocationPolicyParseFailure
                                as i32,
                        },
                    )),
                }))
            }
        };
        runtime.shutdown();
        dispatcher.shutdown();
        // Java: com/google/devtools/build/lib/server/GrpcServerImpl.java:644 GrpcServerImpl.executeCommand
        // PORT: no cookie or command id (the in-process server has no gRPC request ids).
        RunResponse {
            finished: true,
            termination_expected: result.shutdown(),
            exit_code: result.get_exit_code().get_numeric_exit_code(),
            exec_request: result.get_exec_request().cloned(),
            failure_detail: result.get_failure_detail().cloned(),
            // Java: com/google/devtools/build/lib/server/GrpcServerImpl.java:665 GrpcServerImpl.executeCommand
            command_extensions: result.get_response_extensions().to_vec(),
            ..Default::default()
        }
    }

    /// Parses the startup arguments twice: once to read `--option_sources`, then with the right
    /// source for every option.
    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:1280 BlazeRuntime.parseStartupOptions
    fn parse_startup_options(
        modules: &[Arc<dyn BlazeModule>],
        args: &[String],
    ) -> Result<OptionsParsingResult, OptionsParsingException> {
        let option_classes = Self::get_startup_options(modules);

        // First parse the command line so that we get the option_sources argument
        let mut parser = OptionsParser::builder()
            .options_classes(&option_classes)
            .allow_residue(false)
            .build()
            .unwrap_or_else(|e| e.throw());
        parser.parse_with_priority(PriorityCategory::COMMAND_LINE, None, args.to_vec())?;
        let option_sources = parser
            .get_options::<BlazeServerStartupOptions>()
            .expect("BlazeServerStartupOptions")
            .option_sources;
        let option_sources: Vec<(String, String)> = option_sources.0;
        let source_function = move |option: OptionDefinitionRef| -> Option<String> {
            match option_sources
                .iter()
                .find(|(k, _)| k == option.get_option_name())
            {
                None => Some("default".to_string()),
                Some((_, v)) if v.is_empty() => Some("command line".to_string()),
                Some((_, v)) => Some(v.clone()),
            }
        };

        // Then parse the command line again, this time with the correct option sources
        let mut parser = OptionsParser::builder()
            .options_classes(&option_classes)
            .allow_residue(false)
            .build()
            .unwrap_or_else(|e| e.throw());
        parser.parse_with_source_function(
            PriorityCategory::COMMAND_LINE,
            &source_function,
            args.to_vec(),
            None,
        )?;
        Ok(parser)
    }

    /// Creates a new blaze runtime, given the install and output base directories.
    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:1318 BlazeRuntime.newRuntime
    pub(crate) fn new_runtime(
        blaze_modules: Vec<Arc<dyn BlazeModule>>,
        args: &[String],
        abrupt_shutdown_handler: Option<Arc<dyn Fn() + Send + Sync>>,
    ) -> Result<Arc<BlazeRuntime>, NewRuntimeError> {
        let options =
            Self::parse_startup_options(&blaze_modules, args).map_err(NewRuntimeError::Options)?;
        let startup_options = options
            .get_options::<BlazeServerStartupOptions>()
            .expect("BlazeServerStartupOptions");

        // Set up the failure detail path first, so that it can communicate problems with other
        // flags and module initialization.
        let failure_detail_out: Option<PathFragment> = startup_options
            .failure_detail_out
            .as_ref()
            .map(|f| f.0.clone());
        match &failure_detail_out {
            Some(f) if f.is_absolute() => {}
            other => panic!(
                "IllegalArgumentException: Bad --failure_detail_out option specified: '{}'",
                other
                    .as_ref()
                    .map(|f| f.get_path_string().to_string())
                    .unwrap_or_else(|| "null".to_string())
            ),
        }
        CustomFailureDetailPublisher::set_failure_detail_file_path(
            failure_detail_out
                .as_ref()
                .expect("checked above")
                .get_path_string(),
        );

        for module in &blaze_modules {
            module
                .global_init(&options)
                .map_err(NewRuntimeError::Abrupt)?;
        }

        let product_name = startup_options.product_name.to_lowercase();

        let workspace_directory: PathFragment = startup_options.workspace_directory.0.clone();
        let default_system_javabase: PathFragment =
            startup_options.default_system_javabase.0.clone();
        let output_user_root: Option<PathFragment> = startup_options
            .output_user_root
            .as_ref()
            .map(|p| p.0.clone());
        // Never null in Blaze's options (the transcribed field is not optional).
        let install_base: Option<PathFragment> = Some(startup_options.install_base.0.clone());
        let output_base: Option<PathFragment> =
            startup_options.output_base.as_ref().map(|p| p.0.clone());

        // PORT: forceJniLinking is not needed (no JNI).

        // From the point of view of the Java program --install_base, --output_base,
        // --output_user_root, and --failure_detail_out are mandatory options, despite the comment
        // in their declarations.
        let fmt = |p: &Option<PathFragment>| {
            p.as_ref()
                .map(|f| f.get_path_string().to_string())
                .unwrap_or_else(|| "null".to_string())
        };
        if !install_base.as_ref().is_some_and(|p| p.is_absolute()) {
            panic!(
                "IllegalArgumentException: Bad --install_base option specified: '{}'",
                fmt(&install_base)
            );
        }
        if output_user_root.as_ref().is_some_and(|p| !p.is_absolute()) {
            panic!(
                "IllegalArgumentException: Bad --output_user_root option specified: '{}'",
                fmt(&output_user_root)
            );
        }
        if output_base.as_ref().is_some_and(|p| !p.is_absolute()) {
            panic!(
                "IllegalArgumentException: Bad --output_base option specified: '{}'",
                fmt(&output_base)
            );
        }
        #[allow(clippy::unnecessary_literal_unwrap)]
        let install_base = install_base.unwrap();
        let output_base = output_base.expect("--output_base");
        let output_user_root = output_user_root.expect("--output_user_root");
        let exec_root_base = output_base.get_relative_str(ServerDirectories::EXECROOT);

        // The JVM configures java.util.logging from the client's javalog.properties
        // (prefix=<output_base>/java.log) before main runs; the server log is opened here, the
        // first point where the output base is known in-process.
        crate::server_log::init(&Path::create(
            output_base.clone(),
            &(Arc::new(UnixFileSystem::new(blaze_vfs::DigestHashFunction::Sha256))
                as FileSystemRef),
        ));

        // PORT: BazelFileSystemModule.getFileSystem: on Linux the Unix file system with the
        // digest function of --digest_function (its converter only accepts SHA-256, the only
        // function ported) and --unix_digest_hash_attribute_name, which is not ported.
        // Java: com/google/devtools/build/lib/bazel/BazelFileSystemModule.java:90 BazelFileSystemModule.getFileSystem
        if !startup_options.unix_digest_hash_attribute_name.is_empty() {
            blaze_util::unported!(
                "UnixFileSystem.getFastDigest",
                "--unix_digest_hash_attribute_name"
            );
        }
        let fs: FileSystemRef =
            Arc::new(UnixFileSystem::new(blaze_vfs::DigestHashFunction::Sha256));

        // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:1396 BlazeRuntime.newRuntime
        if startup_options.lock_install_base {
            // Acquire a shared lock on the install base to prevent it from being garbage
            // collected by another server while this server is running. Note that the client
            // must already hold a shared lock on the install base at this time (which it will
            // release once it successfully connects to the server), so failure to obtain the lock
            // is not expected.
            // The lock is never released explicitly, so as not to risk releasing it while the
            // install base is still in use. It goes away when the server process dies.
            // PORT: in-process, this process is also the client that holds the shared lock; a
            // second shared lock of the same process on the same file succeeds, and neither is
            // released before the process exits.
            let install_base_path = Path::create(install_base.clone(), &fs);
            let lock_path = install_base_path
                .replace_name(&format!("{}.lock", install_base_path.get_base_name()))
                .expect("the install base is not the root directory");
            match blaze_repo_fetch::file_system_lock::FileSystemLock::try_get(
                &lock_path,
                blaze_repo_fetch::file_system_lock::LockMode::SHARED,
            ) {
                Ok(install_base_lock) => std::mem::forget(install_base_lock),
                Err(e) => {
                    return Err(NewRuntimeError::Abrupt(create_filesystem_exit_exception(
                        &format!(
                            "Failed to acquire shared lock on install base: {}",
                            e.get_message()
                        ),
                        failure_details::filesystem::Code::FailedToLockInstallBase,
                    )));
                }
            }
        }
        // PORT: the event bus exception handlers (--fatal_event_bus_exceptions) serve Guava's
        // EventBus, whose subscribers' exceptions are caught; a panicking subscriber of Blaze's
        // event bus ends the command. Thread.setDefaultUncaughtExceptionHandler: the panic hook
        // (crate::bugreport::install_panic_hook), installed at startup.

        // Set the hook used to display Starlark source lines in a stack trace.
        // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:1452 BlazeRuntime.newRuntime
        {
            let fs = fs.clone();
            blaze_starlark::error::set_source_reader_supplier(Arc::new(move |loc| {
                // TODO(adonovan): opt: cache seen files, as the stack often repeats the same
                // files.
                // fs.getPath throws for a relative path (Path.create: "Paths must be absolute");
                // any failure gives no line.
                let fragment = PathFragment::create(&loc.file);
                if !fragment.is_absolute() {
                    return None;
                }
                let path = Path::create(fragment, &fs);
                // Reading the file as Latin-1 is equivalent to reading raw bytes, which matches
                // Bazel's internal encoding for strings (see StringEncoding).
                let mut bytes = Vec::new();
                std::io::Read::read_to_end(&mut path.get_input_stream().ok()?, &mut bytes).ok()?;
                let content = blaze_util::string_encoding::latin1_decode(&bytes);
                let lines = crate::commands::target_patterns_helper::read_lines(&content);
                let line = loc.line.checked_sub(1)?;
                lines.get(line).map(|l| l.to_string())
            }));
        }

        let workspace_directory_path = if workspace_directory.is_empty() {
            None
        } else {
            Some(Path::create(workspace_directory, &fs))
        };
        let default_system_javabase_path = if default_system_javabase.is_empty() {
            None
        } else {
            Some(Path::create(default_system_javabase, &fs))
        };

        let server_directories = ServerDirectories::new(
            Path::create(install_base, &fs),
            Path::create(output_base, &fs),
            Path::create(output_user_root, &fs),
            Path::create(exec_root_base, &fs),
            Some(startup_options.install_md5.as_str()),
        );

        // Java: BlazeRuntime.Builder.build
        let mut server_builder = ServerBuilder::new();
        for module in &blaze_modules {
            module
                .server_init(&options, &mut server_builder)
                .map_err(NewRuntimeError::Abrupt)?;
        }
        // PORT: blazeStartup, rule classes, package factory, project files and query helpers
        // are not ported.
        let mut runtime = BlazeRuntime {
            file_system: fs,
            blaze_modules,
            command_map: IndexMap::new(),
            info_items: server_builder.get_info_items().clone(),
            startup_options_provider: Arc::new(options),
            product_name,
            workspace: OnceLock::new(),
            env: parking_lot::Mutex::new(None),
            stored_exit_code: parking_lot::Mutex::new(None),
            abrupt_shutdown_handler,
        };
        for command in server_builder.get_commands() {
            runtime.add_command(command.clone());
        }
        // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:261 BlazeRuntime.<init>
        CommandNameCacheInstance::set_command_name_cache(Arc::new(CommandNameCacheImpl::new(
            runtime
                .command_map
                .iter()
                .map(|(name, command)| (name.clone(), command.annotation())),
        )));

        // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:1516 BlazeRuntime.newRuntime
        CustomExitCodePublisher::set_abrupt_exit_status_file_dir(
            server_directories.get_output_base().get_path_string(),
        );
        // Delete the previous file, if any, in case this server is reusing an existing output
        // base from a previous server that had an abrupt exit.
        CustomExitCodePublisher::maybe_delete_abrupt_exit_status_file();

        let directories = BlazeDirectories::new(
            server_directories,
            workspace_directory_path,
            default_system_javabase_path,
            &runtime.product_name.clone(),
        );
        // PORT: BinTools.forProduction is not ported.
        // Keep this line last in this method, so that all other initialization is available to
        // it.
        runtime.init_workspace(directories);
        let runtime = Arc::new(runtime);
        // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:1792 BlazeRuntime.Builder.build
        crate::bugreport::BugReport::set_runtime(runtime.clone());
        Ok(runtime)
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:271 BlazeRuntime.initWorkspace
    fn init_workspace(&mut self, directories: BlazeDirectories) {
        // PORT: module.workspaceInit and the WorkspaceBuilder (Skyframe executor) are not
        // ported.
        let workspace = Arc::new(BlazeWorkspace::new(directories));
        if self.workspace.set(workspace).is_err() {
            panic!("IllegalStateException");
        }
    }
}

/// Exceptions `newRuntime` throws.
pub(crate) enum NewRuntimeError {
    Options(OptionsParsingException),
    Abrupt(AbruptExitException),
}

/// Identity key for an options class (Java compares `Class` objects).
#[derive(Clone, Copy)]
pub(crate) struct ClassKey(pub OptionsClassRef);

impl PartialEq for ClassKey {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.0, other.0)
    }
}
impl Eq for ClassKey {}
impl std::hash::Hash for ClassKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (self.0 as *const _ as usize).hash(state);
    }
}

/// `batchMain`'s execution of the `run` command's `ExecRequest` as a child process (a
/// `ProcessBuilder` with inherited I/O), whose exit code it returns.
// Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:1152 BlazeRuntime.batchMain
impl crate::bugreport::BlazeRuntimeInterface for BlazeRuntime {
    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:1616 BlazeRuntime.getProductName
    fn get_product_name(&self) -> String {
        self.product_name.clone()
    }

    /// PORT: no heap dump (`--heap_dump_on_oom`): Blaze has no out-of-memory crash (see
    /// crate::bugreport::CrashFailureDetails::for_throwable).
    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:1621 BlazeRuntime.fillInCrashContext
    fn fill_in_crash_context(&self, ctx: &mut crate::bugreport::CrashContext) {
        let Some(local_env) = self.env.lock().clone() else {
            return;
        };
        // PORT: options.oomMessage only reaches the message of an out-of-memory crash, which
        // Blaze does not have.
        ctx.reporting_to(local_env.reporter);
    }

    /// PORT: Blaze's `SkyframeExecutor` lives in the command, so the crash events go to the
    /// command's event bus (`postLoggingStatsWhenCrashing` only posts statistics for logging).
    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:675 BlazeRuntime.cleanUpForCrash
    fn clean_up_for_crash(&self, exit_code: &crate::exit_code::DetailedExitCode) {
        if self.declare_exit_code(exit_code) {
            let local_env = self.env.lock().clone();
            if let Some(local_env) = &local_env {
                local_env.notify_on_crash(format!(
                    "{} is crashing: {}",
                    self.product_name,
                    exit_code
                        .get_failure_detail()
                        .map(|f| f.message.as_str())
                        .unwrap_or("")
                ));
            }
            // Only try to publish events if we won the exit code race. Otherwise someone else is
            // already exiting for us.
            if let Some(local_env) = local_env {
                use blaze_events::EventBus;
                local_env
                    .event_bus
                    .post(Arc::new(crate::bugreport::CrashEvent));
                local_env
                    .event_bus
                    .post(Arc::new(crate::bugreport::CommandCompleteEvent::new(
                        exit_code.clone(),
                    )));
            }
        }
        // We don't call #shutDown() here because all it does is shut down the modules, and who
        // knows if they can be trusted. Instead, we call runtime#shutdownOnCrash() which attempts
        // to cleanly shut down those modules that might have something pending to do as a
        // best-effort operation.
        self.shut_down_modules_on_crash(exit_code);
    }
}

// Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:1928 BlazeRuntime.readPidFile
fn read_pid_file(pid_file: &Path) -> Result<i32, AbruptExitException> {
    // FileSystemUtils.readContentAsLatin1
    let mut bytes = Vec::new();
    let read = pid_file
        .get_input_stream()
        .and_then(|mut input| std::io::Read::read_to_end(&mut input, &mut bytes));
    if let Err(e) = read {
        // PORT: the message of Rust's io::Error stands for the IOException's.
        return Err(create_filesystem_exit_exception(
            &format!("Server pid file read failed: {}", e),
            failure_details::filesystem::Code::ServerPidTxtFileReadFailure,
        ));
    }
    let contents = blaze_util::string_encoding::latin1_decode(&bytes);
    // Integer.parseInt: an optional sign and ASCII digits.
    match contents.parse::<i32>() {
        Ok(pid) => Ok(pid),
        // Invalid contents (not a number) is more likely than not a filesystem issue.
        Err(_) => Err(create_filesystem_exit_exception(
            // NumberFormatException.forInputString(s, 10).getMessage()
            &format!(
                "Server pid file corrupted: For input string: \"{}\"",
                contents
            ),
            failure_details::filesystem::Code::ServerPidTxtFileReadFailure,
        )),
    }
}

// Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:1539 BlazeRuntime.createFilesystemExitException
fn create_filesystem_exit_exception(
    message: &str,
    detailed_code: failure_details::filesystem::Code,
) -> AbruptExitException {
    AbruptExitException::new(DetailedExitCode::of(FailureDetail {
        message: message.to_string(),
        category: Some(failure_detail::Category::Filesystem(
            failure_details::Filesystem {
                code: detailed_code as i32,
            },
        )),
    }))
}

/// The pids of this process's children (`ProcessHandle.current().children()`), from
/// `/proc/self/task/<tid>/children`.
fn child_processes() -> Vec<libc::pid_t> {
    let mut children = Vec::new();
    let Ok(tasks) = std::fs::read_dir("/proc/self/task") else {
        return children;
    };
    for task in tasks.flatten() {
        let Ok(list) = std::fs::read(task.path().join("children")) else {
            continue;
        };
        for pid in list.split(|b| b.is_ascii_whitespace()) {
            if let Ok(pid) = std::str::from_utf8(pid)
                .unwrap_or("")
                .parse::<libc::pid_t>()
            {
                children.push(pid);
            }
        }
    }
    children
}

fn run_exec_request_in_batch_mode(request: &ExecRequest) -> i32 {
    use blaze_util::string_encoding::{internal_to_platform, latin1_decode};
    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:1215 BlazeRuntime.internalBytesToPlatformString
    // (the bytes are an internal string's; the platform string is the same bytes).
    let internal = |bytes: &[u8]| latin1_decode(bytes).into_owned();
    let argv: Vec<String> = request.argv.iter().map(|a| internal(a)).collect();
    let working_directory = internal(&request.working_directory);
    if argv.is_empty() {
        // ProcessBuilder.start: IndexOutOfBoundsException on an empty command.
        panic!("IndexOutOfBoundsException: ProcessBuilder.start with an empty command");
    }
    let mut process = std::process::Command::new(&*internal_to_platform(&argv[0]));
    process
        .args(
            argv[1..]
                .iter()
                .map(|a| internal_to_platform(a).into_owned()),
        )
        .current_dir(&*internal_to_platform(&working_directory));
    for name in &request.environment_variable_to_clear {
        process.env_remove(&*internal_to_platform(&internal(name)));
    }
    for variable in &request.environment_variable {
        process.env(
            &*internal_to_platform(&internal(&variable.name)),
            &*internal_to_platform(&internal(&variable.value)),
        );
    }
    match process.status() {
        // Process.waitFor: the exit status, or 0x80 + the signal number.
        Ok(status) => {
            use std::os::unix::process::ExitStatusExt as _;
            status
                .code()
                .unwrap_or_else(|| 0x80 + status.signal().unwrap_or(0))
        }
        Err(e) => {
            // We are in batch mode, thus, stdout/stderr are the same as that of the client.
            // The message of the JDK's ProcessBuilder.start IOException.
            let message = format!(
                "Cannot run program \"{}\" (in directory \"{}\"): error={}, {}",
                argv[0],
                working_directory,
                e.raw_os_error().unwrap_or(0),
                strerror(e.raw_os_error().unwrap_or(0))
            );
            eprintln!("Cannot execute process for 'run' command: {message}");
            ExitCode::LOCAL_ENVIRONMENTAL_ERROR.get_numeric_exit_code()
        }
    }
}

/// The C library's text of an errno value (the JDK's `error=N, <strerror>` message part).
fn strerror(errno: i32) -> String {
    // SAFETY: strerror returns a pointer to a NUL-terminated static or thread-local string.
    let message = unsafe { std::ffi::CStr::from_ptr(libc::strerror(errno)) };
    blaze_util::string_encoding::latin1_decode(message.to_bytes()).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/BlazeRuntimeTest.java:115 BlazeRuntimeTest.optionSplitting
    #[test]
    fn option_splitting() {
        let (startup, other) = BlazeRuntime::split_startup_options(
            &[],
            &s(&[
                "--install_base=/foo --host_jvm_args=-Xmx1B",
                "build",
                "//foo:bar",
                "--nobuild",
            ]),
        );
        assert_eq!(startup, s(&["--install_base=/foo --host_jvm_args=-Xmx1B"]));
        assert_eq!(other, s(&["build", "//foo:bar", "--nobuild"]));
    }

    // A regression test to make sure that the 'no' prefix is handled correctly.
    // Java: src/test/java/com/google/devtools/build/lib/runtime/BlazeRuntimeTest.java:130 BlazeRuntimeTest.optionSplittingNoPrefix
    #[test]
    fn option_splitting_no_prefix() {
        let (startup, other) =
            BlazeRuntime::split_startup_options(&[], &s(&["--nobatch", "build"]));
        assert_eq!(startup, s(&["--nobatch"]));
        assert_eq!(other, s(&["build"]));
    }

    #[test]
    #[ignore = "needs the profiler (manageProfiles)"]
    fn manage_profiles() {}

    #[test]
    #[ignore = "needs BlazeRuntime.afterCommand as a runtime method (Blaze's is the dispatcher's after_command) and a CommandEnvironment built outside the dispatcher; the same behaviour runs in dispatcher_tests crash_prevents_new_command"]
    fn crash_test() {}

    #[test]
    #[ignore = "needs response extensions"]
    fn adds_response_extensions() {}

    #[test]
    #[ignore = "needs idle tasks"]
    fn adds_gc_and_interner_shrinking_idle_task_no_state_kept_after_build() {}

    #[test]
    #[ignore = "needs idle tasks"]
    fn adds_gc_and_interner_shrinking_idle_task_state_kept_after_build() {}

    #[test]
    #[ignore = "needs idle tasks"]
    fn does_not_add_install_base_gc_idle_task_when_disabled() {}

    #[test]
    #[ignore = "needs idle tasks"]
    fn adds_install_base_gc_idle_task_when_enabled() {}

    #[test]
    #[ignore = "needs idle tasks"]
    fn does_not_add_action_cache_gc_idle_task_when_disabled() {}

    #[test]
    #[ignore = "needs idle tasks"]
    fn adds_action_cache_gc_idle_task_when_enabled() {}

    #[test]
    #[ignore = "needs idle tasks"]
    fn adds_idle_tasks_from_modules() {}

    #[test]
    #[ignore = "needs a BlazeRuntime test builder with fake modules"]
    fn adds_commands_from_modules() {}

    #[test]
    #[ignore = "needs BlazeService"]
    fn returns_both_modules_and_services_as_options_suppliers() {}
}
