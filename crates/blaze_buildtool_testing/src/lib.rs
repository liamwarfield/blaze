//! Test support: the Rust counterpart of Java's `BuildIntegrationTestCase`
//! (`src/test/java/com/google/devtools/build/lib/buildtool/util/BuildIntegrationTestCase.java`)
//! and of the parts of its `BlazeRuntimeWrapper` the tests use.
//!
//! A [`BuildIntegrationTestCase`] is a workspace on disk (a `MODULE.bazel` with a dependency on
//! `platforms`) and one in-process [`BlazeRuntime`]
//! (Bazel's modules, as the `blaze` binary builds it) with one [`BlazeCommandDispatcher`]: every
//! command of a test runs through the dispatcher on that runtime, so the server state a command
//! leaves (the Skyframe graph kept between commands, the action cache, the output base) is there
//! for the next command and for the test's assertions, as in Java's server.
//!
//! Differences from the Java fixture (each also marked `PORT:` where it applies):
//!
//! - **Commands go through the dispatcher.** Java's `BlazeRuntimeWrapper` builds a
//!   `CommandEnvironment` and calls the command directly ("not exercising the command
//!   dispatcher"); here [`BuildIntegrationTestCase::build_target`] and every other command run
//!   through `BlazeCommandDispatcher.exec`, with the fixture's options on the command line. The
//!   options reach the command as Java's `runtimeWrapper.getOptions()` do (Java's
//!   `ConfigCommandTest.analyzeTarget` passes them to the dispatcher the same way).
//! - **Real tools.** Java writes mock tools into the workspace and overrides `@bazel_tools` with
//!   them (`AnalysisMock.setupMockToolsRepository`, `--override_repository=bazel_tools=...`);
//!   here the install base is the oracle's (its `embedded_tools` is the real `@bazel_tools`),
//!   and external modules come through the shared repository cache, as in the binary.
//! - **Events.** Java's `EventCollector` is attached to the runtime's reporter; here a module
//!   attaches a collector to each command's reporter (`beforeCommand`), and the command's
//!   stdout and stderr are also recorded ([`BuildIntegrationTestCase::out_err`]).
//! - **The Skyframe executor.** Java's `getSkyframeExecutor()` is the server's one executor;
//!   Blaze builds an executor per command and keeps its graph between commands
//!   ([`blaze_sky_top::incremental`]). [`BuildIntegrationTestCase::get_kept_evaluator`] is that
//!   graph after the last command; [`BuildIntegrationTestCase::on_before_command`] reaches a
//!   command's environment (and through it the executor) while the command runs.
//! - **One runtime per process at a time.** The runtime sets process-wide state (the custom
//!   exit code files, the crash handling); fixtures of one test binary are serialized.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use blaze_events::io::RecordingOutErr;
use blaze_events::{Event, EventHandler, EventKind};
use blaze_protos::blaze::invocation_policy::InvocationPolicy;
use blaze_runtime::command::BlazeCommandResult;
use blaze_runtime::command_environment::CommandEnvironment;
use blaze_runtime::dispatcher::{BlazeCommandDispatcher, LockingMode, UiVerbosity};
use blaze_runtime::module::{AbruptExitException, BlazeModule};
use blaze_runtime::runtime::BlazeRuntime;
use blaze_runtime::workspace::BlazeWorkspace;
use blaze_skyframe::Evaluator;
use parking_lot::{Mutex, MutexGuard};

/// The md5 of the oracle's install base (`UPSTREAM.json`'s binary).
pub const ORACLE_INSTALL_MD5: &str = "58cf1ecbff441bac762b0050d6da21e9";

/// Fixtures of one process run one at a time (the runtime sets process-wide state).
static SERIAL: Mutex<()> = Mutex::new(());

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").expect("HOME"))
}

/// The oracle's install base: its `embedded_tools` is `@bazel_tools`.
pub fn oracle_install_base() -> PathBuf {
    home().join(format!(
        ".cache/blaze/oracle-out/ur/install/{ORACLE_INSTALL_MD5}"
    ))
}

/// A hook run in every command's `beforeCommand`.
pub type BeforeCommandHook = Arc<dyn Fn(&mut CommandEnvironment) + Send + Sync>;

/// Collects the events of every command (Java's `EventCollector`).
#[derive(Default)]
struct EventCollector {
    events: Mutex<Vec<Event>>,
}

impl EventHandler for EventCollector {
    fn handle(&self, event: &Event) {
        self.events.lock().push(event.clone());
    }
}

/// The fixture's module: attaches the event collector to each command's reporter and runs the
/// test's `beforeCommand` hooks.
struct TestModule {
    collector: Arc<EventCollector>,
    hooks: Arc<Mutex<Vec<BeforeCommandHook>>>,
}

impl BlazeModule for TestModule {
    fn name(&self) -> &'static str {
        "BuildIntegrationTestCase.TestModule"
    }

    fn before_command(&self, env: &mut CommandEnvironment) -> Result<(), AbruptExitException> {
        env.get_reporter().add_handler(self.collector.clone());
        let hooks: Vec<BeforeCommandHook> = self.hooks.lock().clone();
        for hook in hooks {
            hook(env);
        }
        Ok(())
    }
}

/// A workspace and one in-process runtime that runs several commands (see the module
/// documentation).
// Java: src/test/java/com/google/devtools/build/lib/buildtool/util/BuildIntegrationTestCase.java:150 BuildIntegrationTestCase
pub struct BuildIntegrationTestCase {
    runtime: Arc<BlazeRuntime>,
    dispatcher: Arc<BlazeCommandDispatcher>,
    workspace: PathBuf,
    output_base: PathBuf,
    /// `runtimeWrapper.getOptions()`.
    options: Vec<String>,
    collector: Arc<EventCollector>,
    hooks: Arc<Mutex<Vec<BeforeCommandHook>>>,
    /// The stdout and stderr of the last command.
    out_err: RecordingOutErr,
    last_result: Option<BlazeCommandResult>,
    _dir: tempfile::TempDir,
    _serial: MutexGuard<'static, ()>,
}

impl BuildIntegrationTestCase {
    /// `createFilesAndMocks`, `createRuntimeWrapper` and `setupOptions`: a scratch workspace
    /// (with an empty `MODULE.bazel`), an output base, and the runtime.
    // Java: src/test/java/com/google/devtools/build/lib/buildtool/util/BuildIntegrationTestCase.java:240 BuildIntegrationTestCase.createFilesAndMocks
    pub fn new() -> BuildIntegrationTestCase {
        let serial = SERIAL.lock();
        let install_base = oracle_install_base();
        assert!(
            install_base.join("embedded_tools").is_dir(),
            "BuildIntegrationTestCase needs the oracle's install base at {} (run the oracle once, \
             see scripts/oracle)",
            install_base.display()
        );
        // The runtime's hook records panics for its crash handling; the default hook still
        // prints, so that a failing assertion shows its message.
        static PANIC_HOOK: std::sync::Once = std::sync::Once::new();
        PANIC_HOOK.call_once(|| {
            let default_hook = std::panic::take_hook();
            blaze_runtime::bugreport::install_panic_hook();
            let runtime_hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                default_hook(info);
                runtime_hook(info);
            }));
        });
        // Bazel.main, once per process.
        static BUILD_INFO: std::sync::Once = std::sync::Once::new();
        BUILD_INFO.call_once(|| {
            blaze_runtime::version_info::BlazeVersionInfo::set_build_info(
                blaze_runtime::version_info::try_get_build_info(),
            )
        });

        let dir = tempfile::tempdir().expect("temporary directory");
        let root = dir.path().to_path_buf();
        let workspace = root.join("workspace");
        let output_base = root.join("output_base");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&output_base).unwrap();
        // PORT: Java's mock client writes a MODULE.bazel whose deps make `@platforms` visible
        // from the main repository (TestConstants.PRODUCT_SPECIFIC_FLAGS name
        // `@platforms//host`); here it is the real module, at the version `@bazel_tools` depends
        // on (embedded_tools/MODULE.bazel), so it comes from the shared repository cache.
        std::fs::write(
            workspace.join("MODULE.bazel"),
            "bazel_dep(name = \"platforms\", version = \"1.0.0\")\n",
        )
        .unwrap();

        let collector = Arc::new(EventCollector::default());
        let hooks: Arc<Mutex<Vec<BeforeCommandHook>>> = Arc::new(Mutex::new(Vec::new()));
        let mut modules = blaze_runtime::bazel_modules::bazel_modules();
        modules.push(Arc::new(TestModule {
            collector: collector.clone(),
            hooks: hooks.clone(),
        }));
        let args = vec![
            format!("--install_base={}", install_base.display()),
            format!("--install_md5={ORACLE_INSTALL_MD5}"),
            format!("--output_base={}", output_base.display()),
            format!("--output_user_root={}", root.display()),
            format!("--workspace_directory={}", workspace.display()),
            format!(
                "--failure_detail_out={}",
                output_base.join("failure_detail.rawproto").display()
            ),
        ];
        let runtime = match BlazeRuntime::new_runtime(modules, &args, None) {
            Ok(runtime) => runtime,
            Err(_) => panic!("BlazeRuntime.newRuntime failed"),
        };
        let dispatcher = Arc::new(BlazeCommandDispatcher::new(
            runtime.clone(),
            std::process::id() as i32,
        ));
        let mut fixture = BuildIntegrationTestCase {
            runtime,
            dispatcher,
            workspace,
            output_base,
            options: Vec::new(),
            collector,
            hooks,
            out_err: RecordingOutErr::new(),
            last_result: None,
            _dir: dir,
            _serial: serial,
        };
        fixture.setup_options();
        fixture
    }

    /// `setupOptions`: the options of every command.
    ///
    /// PORT: Java's `--override_repository=bazel_tools=embedded_tools` points at the mock tools
    /// it writes into the workspace; here `@bazel_tools` is the install base's. The shared
    /// repository cache, `--color=no` and `--curses=no` are added (no terminal).
    // Java: src/test/java/com/google/devtools/build/lib/buildtool/util/BuildIntegrationTestCase.java:655 BuildIntegrationTestCase.setupOptions
    fn setup_options(&mut self) {
        self.reset_options();
        self.add_options(&[
            // Set visibility to public so that test cases don't have to bother with visibility
            // declarations
            "--default_visibility=public",
            // Don't show progress messages unless we need to, to keep the noise down.
            "--noshow_progress",
            // Don't use ijars, because we don't have the executable in these tests
            "--nouse_ijars",
        ]);
        self.add_options(&["--experimental_extended_sanity_checks"]);
        // TestConstants.PRODUCT_SPECIFIC_FLAGS
        self.add_options(&[
            "--platforms=@platforms//host",
            "--host_platform=@platforms//host",
            "--incompatible_use_toolchain_resolution_for_java_rules",
            "--incompatible_disable_select_on=cpu,crosstool_top,host_cpu",
        ]);
        // TestConstants.PRODUCT_SPECIFIC_BUILD_LANG_OPTIONS
        self.add_options(&["--incompatible_autoload_externally="]);
        let repository_cache = home().join(".cache/blaze/repo-cache");
        self.add_options(&[
            &format!("--repository_cache={}", repository_cache.display()),
            "--color=no",
            "--curses=no",
        ]);
    }

    /// `resetOptions`.
    // Java: src/test/java/com/google/devtools/build/lib/buildtool/util/BuildIntegrationTestCase.java:691 BuildIntegrationTestCase.resetOptions
    pub fn reset_options(&mut self) {
        self.options.clear();
    }

    /// `addOptions(args...)`: options of every later command that accepts them.
    // Java: src/test/java/com/google/devtools/build/lib/buildtool/util/BuildIntegrationTestCase.java:695 BuildIntegrationTestCase.addOptions
    pub fn add_options(&mut self, args: &[&str]) {
        self.options.extend(args.iter().map(|a| a.to_string()));
    }

    /// `runtimeWrapper.getOptions()`.
    pub fn get_options(&self) -> &[String] {
        &self.options
    }

    /// Runs `hook` in the `beforeCommand` of every later command (Java tests reach the command's
    /// environment through `runtimeWrapper` and `injectListenerAtStartOfNextBuild`).
    pub fn on_before_command(
        &self,
        hook: impl Fn(&mut CommandEnvironment) + Send + Sync + 'static,
    ) {
        self.hooks.lock().push(Arc::new(hook));
    }

    /// `write(relativePath, lines...)`: a workspace file, each line ending in `\n`.
    // Java: src/test/java/com/google/devtools/build/lib/buildtool/util/BuildIntegrationTestCase.java:963 BuildIntegrationTestCase.write
    pub fn write(&self, relative_path: &str, lines: &[&str]) -> PathBuf {
        let path = self.workspace.join(relative_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let mut content = String::new();
        for line in lines {
            content.push_str(line);
            content.push('\n');
        }
        std::fs::write(&path, content).unwrap();
        path
    }

    /// `buildTarget(targets...)`: `build` with the fixture's options and `targets`.
    ///
    /// PORT: Java returns the `BuildResult` and throws on a failed build; here the command's
    /// result is returned and the test checks it (the build's events and output are in
    /// [`Self::events`] and [`Self::out_err`]).
    // Java: src/test/java/com/google/devtools/build/lib/buildtool/util/BuildIntegrationTestCase.java:853 BuildIntegrationTestCase.buildTarget
    pub fn build_target(&mut self, targets: &[&str]) -> &BlazeCommandResult {
        let mut args: Vec<String> = vec!["build".to_owned()];
        args.extend(self.options.iter().cloned());
        args.extend(targets.iter().map(|t| t.to_string()));
        self.exec_args(args)
    }

    /// `dispatcher.exec(args, "test", outErr)` with `args` as given (no fixture options): Java
    /// tests that call the dispatcher themselves pass the options they want.
    pub fn dispatch(&mut self, args: &[&str]) -> &BlazeCommandResult {
        self.exec_args(args.iter().map(|a| a.to_string()).collect())
    }

    /// Runs `command` with the fixture's options followed by `args`.
    pub fn run_command(&mut self, command: &str, args: &[&str]) -> &BlazeCommandResult {
        let mut all: Vec<String> = vec![command.to_owned()];
        all.extend(self.options.iter().cloned());
        all.extend(args.iter().map(|a| a.to_string()));
        self.exec_args(all)
    }

    fn exec_args(&mut self, mut args: Vec<String>) -> &BlazeCommandResult {
        // The client's working directory (the client sends it with every command).
        args.push(format!("--client_cwd={}", self.workspace.display()));
        self.out_err = RecordingOutErr::new();
        let result = self
            .dispatcher
            .exec(
                InvocationPolicy::default(),
                args,
                self.out_err.out_err().clone(),
                LockingMode::WAIT,
                UiVerbosity::NORMAL,
                "test",
                None,
            )
            .expect("not interrupted while waiting for the command lock");
        self.last_result = Some(result);
        self.last_result.as_ref().unwrap()
    }

    /// The stdout and stderr of the last command.
    pub fn out_err(&self) -> &RecordingOutErr {
        &self.out_err
    }

    /// `runtimeWrapper.getLastCommandResult()`.
    pub fn get_last_command_result(&self) -> Option<&BlazeCommandResult> {
        self.last_result.as_ref()
    }

    /// The events of all commands so far.
    pub fn events(&self) -> Vec<Event> {
        self.collector.events.lock().clone()
    }

    /// Forgets the collected events (`events.clear()`).
    pub fn clear_events(&self) {
        self.collector.events.lock().clear();
    }

    /// `assertContainsEvent(expectedEvent)`: an error, warning or info event containing it.
    // Java: src/test/java/com/google/devtools/build/lib/buildtool/util/BuildIntegrationTestCase.java:1227 BuildIntegrationTestCase.assertContainsEvent
    pub fn assert_contains_event(&self, expected_event: &str) -> Event {
        let events = self.events();
        events
            .iter()
            .find(|e| {
                matches!(
                    e.get_kind(),
                    EventKind::Error | EventKind::Warning | EventKind::Info
                ) && e.get_message().contains(expected_event)
            })
            .cloned()
            .unwrap_or_else(|| {
                panic!(
                    "no event containing {expected_event:?} among {:?}",
                    events.iter().map(|e| e.get_message()).collect::<Vec<_>>()
                )
            })
    }

    /// `assertDoesNotContainEvent(unexpectedEvent)`.
    // Java: src/test/java/com/google/devtools/build/lib/buildtool/util/BuildIntegrationTestCase.java:1241 BuildIntegrationTestCase.assertDoesNotContainEvent
    pub fn assert_does_not_contain_event(&self, unexpected_event: &str) {
        let events = self.events();
        if let Some(e) = events
            .iter()
            .find(|e| e.get_message().contains(unexpected_event))
        {
            panic!("unexpected event {:?}", e.get_message());
        }
    }

    /// `getRuntime()`.
    pub fn get_runtime(&self) -> &Arc<BlazeRuntime> {
        &self.runtime
    }

    /// `getBlazeWorkspace()`.
    pub fn get_blaze_workspace(&self) -> &Arc<BlazeWorkspace> {
        self.runtime.get_workspace()
    }

    /// The Skyframe graph the last command left for the next one (Java `getSkyframeExecutor()`'s
    /// evaluator after the command); `None` when the server kept none.
    pub fn get_kept_evaluator(&self) -> Option<Arc<Evaluator>> {
        self.runtime
            .get_workspace()
            .get_incremental_state()
            .kept_evaluator()
    }

    /// `getWorkspace()`.
    pub fn get_workspace(&self) -> &Path {
        &self.workspace
    }

    /// `getOutputBase()`.
    pub fn get_output_base(&self) -> &Path {
        &self.output_base
    }
}

impl Default for BuildIntegrationTestCase {
    fn default() -> Self {
        Self::new()
    }
}
