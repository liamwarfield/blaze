//! `BlazeRuntime`: the Blaze server, its startup (`main`, `batchMain`, `serverMain`,
//! `newRuntime`) and its module, command and info-item registries.

use std::sync::{Arc, OnceLock};

use blaze_events::OutErr;
use blaze_optdefs::command::blaze_server_startup_options::BlazeServerStartupOptions;
use blaze_options::definition::{OptionDefinitionRef, OptionsClassRef};
use blaze_options::{OptionsParser, OptionsParsingException, PriorityCategory};
use blaze_protos::command_server::RunRequest;
use blaze_vfs::{FileSystemRef, Path, PathFragment, UnixFileSystem};
use indexmap::IndexMap;

use crate::command::BlazeCommand;
use crate::directories::{BlazeDirectories, ServerDirectories};
use crate::dispatcher::{BlazeCommandDispatcher, LockingMode, UiVerbosity};
use crate::exit_code::ExitCode;
use crate::info_item::InfoItem;
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
    pub(crate) fn before_command(&self) {}

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

    /// Main method for the Blaze server startup.
    ///
    /// In-process seam: Java's `main` either runs `batchMain` (args start with `--batch`) or
    /// `serverMain` and serves gRPC requests. Blaze's client calls [`BlazeRuntime::batch_main`]
    /// or [`BlazeRuntime::server_main_single_command`] directly.
    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:911 BlazeRuntime.main
    pub fn main(modules: Vec<Arc<dyn BlazeModule>>, args: Vec<String>) -> i32 {
        // blaze.cc will put --batch first if the user set it.
        if args.first().map(String::as_str) == Some("--batch") {
            // Run Blaze in batch mode.
            return Self::batch_main(modules, args);
        }
        blaze_util::unported!("BlazeRuntime.serverMain", "gRPC server mode (wave W8)")
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
    pub fn batch_main(modules: Vec<Arc<dyn BlazeModule>>, args: Vec<String>) -> i32 {
        // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:1087 BlazeRuntime.batchMain
        let _signal_handler = crate::interrupt::capture_sigint();
        let (startup_args, other_args) = Self::split_startup_options(&modules, &args);

        let runtime = match Self::new_runtime(modules, &startup_args) {
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
        // PORT: InvocationPolicyParser.parsePolicy(startupOptions.invocationPolicy) is not
        // ported; a non-empty policy fails loudly.
        if !startup_options.invocation_policy.is_empty() {
            blaze_util::unported!("InvocationPolicyParser.parsePolicy");
        }

        let startup_options_from_command_line: Vec<(String, String)> = startup_args
            .iter()
            .map(|option| (String::new(), option.clone()))
            .collect();

        let dispatcher = BlazeCommandDispatcher::new(
            runtime.clone(),
            BlazeCommandDispatcher::UNKNOWN_SERVER_PID,
        );
        let result = dispatcher.exec(
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
        );
        // PORT: result.getExecRequest() (the `run` command) is not ported.
        runtime.shutdown();
        result.get_exit_code().get_numeric_exit_code()
    }

    /// Server mode for exactly one command: `serverMain` creates the runtime and the gRPC
    /// server, and `GrpcServerImpl.run` executes the request (`executeCommand`). The in-process
    /// server then shuts down.
    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:1200 BlazeRuntime.serverMain
    pub fn server_main_single_command(
        modules: Vec<Arc<dyn BlazeModule>>,
        startup_args: Vec<String>,
        request: RunRequest,
    ) -> i32 {
        let runtime = match Self::new_runtime(modules, &startup_args) {
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
        // server.pid was written in the C++ launcher; in-process the server is this process.
        let server_pid = std::process::id() as i32;
        // The server is a daemon (ExecuteDaemon: setsid), so its children have no controlling
        // terminal and a Ctrl-C reaches only the client, which cancels the command
        // (blaze_client::signal_handler). In-process, the children get a session of their own.
        // Java: src/main/tools/daemonize.cc:270 main
        blaze_shell::java_process::set_start_children_in_new_session(true);
        let dispatcher = BlazeCommandDispatcher::new(runtime.clone(), server_pid);

        // Java: com/google/devtools/build/lib/server/GrpcServerImpl.java:539 GrpcServerImpl.executeCommand
        // PORT: the request cookie, cancellation, preemption and invocation policy are not
        // ported; output streams go straight to this process's stdout/stderr.
        if !request.invocation_policy.is_empty() {
            blaze_util::unported!("InvocationPolicyParser.parsePolicy");
        }
        let args: Vec<String> = request
            .arg
            .iter()
            .map(|a| String::from_utf8(a.clone()).expect("non-UTF-8 argument"))
            .collect();
        let startup_options_tagged_with_bazel_rc: Vec<(String, String)> = request
            .startup_options
            .iter()
            .map(|o| {
                (
                    String::from_utf8_lossy(&o.source).into_owned(),
                    String::from_utf8_lossy(&o.option).into_owned(),
                )
            })
            .collect();
        let result = dispatcher.exec(
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
        );
        runtime.shutdown();
        result.get_exit_code().get_numeric_exit_code()
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
    fn new_runtime(
        blaze_modules: Vec<Arc<dyn BlazeModule>>,
        args: &[String],
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
        // PORT: CustomFailureDetailPublisher.setFailureDetailFilePath: the failure detail file
        // carries the detail of an abrupt server exit to a client whose gRPC connection died;
        // the in-process server hands its exit code to the client directly.

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

        // PORT: the shared install base lock (--lock_install_base,
        // FileSystemLock.tryGet(<install_base>.lock, SHARED)) is not taken again here: the client
        // holds both file locks, the shared install base lock and the exclusive output base lock,
        // from `AcquireLock` until the process exits (blaze_client `BlazeServer::communicate`
        // runs the command in-process without releasing them), and those client locks are what
        // excludes a second Blaze process.
        // PORT: the event bus exception handlers (--fatal_event_bus_exceptions) serve Guava's
        // EventBus, whose subscribers' exceptions are caught; a panicking subscriber of Blaze's
        // event bus ends the command. The Starlark source-line hook belongs to the Java
        // interpreter's tracebacks.

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
        };
        for command in server_builder.get_commands() {
            runtime.add_command(command.clone());
        }

        // PORT: CustomExitCodePublisher (the abrupt exit status file) tells a client whose gRPC
        // connection died which exit code to use; the in-process server returns it directly.

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
        Ok(Arc::new(runtime))
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
enum NewRuntimeError {
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
    #[ignore = "needs BugReporter crash handling"]
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
