//! Ports of `src/test/java/com/google/devtools/build/lib/runtime/BlazeCommandDispatcherTest.java`
//! and `CommandInterruptionTest.java`: commands run through one runtime and one dispatcher,
//! including the server's command lock (`LockingMode.WAIT`), the shutdown reason after a crash,
//! and the interruption of a command's thread.
//!
//! The runtime is built by `BlazeRuntime::new_runtime` from startup options naming scratch
//! directories (Java: `BlazeRuntime.Builder` with `ServerDirectories` of scratch dirs), with
//! test modules that register the test commands (`runtime.overrideCommands`).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier, mpsc};
use std::time::Duration;

use blaze_events::io::RecordingOutErr;
use blaze_options::OptionsClass as _;
use blaze_options::definition::OptionsClassRef;
use blaze_options::rc::handler::Command as RcCommand;
use blaze_protos::blaze::invocation_policy::InvocationPolicy;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail::Category};
use blaze_server::interruptible::{self, Monitor};
use blaze_shell::thread_interrupt::{InterruptedException, ThreadInterrupt};
use parking_lot::{Mutex, MutexGuard};

use crate::bugreport::BlazeRuntimeInterface as _;
use crate::command::{BlazeCommand, BlazeCommandResult, BuildPhase, CommandAnnotation};
use crate::command_environment::CommandEnvironment;
use crate::custom_exit_code_publisher::{
    CustomExitCodePublisher, CustomFailureDetailPublisher, TEST_SERIAL,
};
use crate::dispatcher::{BlazeCommandDispatcher, LockingMode, UiVerbosity};
use crate::exit_code::{DetailedExitCode, ExitCode};
use crate::module::{AbruptExitException, BlazeModule, ServerBuilder};
use crate::options_result::OptionsParsingResult;
use crate::runtime::BlazeRuntime;

mod test_options {
    use blaze_options::*;

    options! {
        /// Options for `FooCommand`.
        // Java: src/test/java/com/google/devtools/build/lib/runtime/BlazeCommandDispatcherTest.java:150 BlazeCommandDispatcherTest.FooOptions
        pub struct FooOptions {
            #[option(
                name = "success",
                default_value = "true",
                documentation_category = UNCATEGORIZED,
                effect_tags = [NO_OP],
                help = "",
            )]
            pub exit_status: bool,

            #[option(
                name = "stdout",
                default_value = "",
                documentation_category = UNCATEGORIZED,
                effect_tags = [NO_OP],
                help = "",
            )]
            pub stdout: String,

            #[option(
                name = "stderr",
                default_value = "",
                documentation_category = UNCATEGORIZED,
                effect_tags = [NO_OP],
                help = "",
            )]
            pub stderr: String,
        }
    }

    options! {
        /// Options class to pass configuration to our dummy wait command.
        // Java: src/test/java/com/google/devtools/build/lib/runtime/CommandInterruptionTest.java:93 CommandInterruptionTest.WaitOptions
        pub struct WaitOptions {
            #[option(
                name = "expect_interruption",
                default_value = "false",
                documentation_category = UNCATEGORIZED,
                effect_tags = [NO_OP],
                help = "",
            )]
            pub expect_interruption: bool,
        }
    }
}

use test_options::{FooOptions, WaitOptions};

fn no_options() -> Vec<OptionsClassRef> {
    Vec::new()
}

fn foo_options() -> Vec<OptionsClassRef> {
    vec![FooOptions::class()]
}

fn wait_options() -> Vec<OptionsClassRef> {
    vec![WaitOptions::class()]
}

/// A `@Command(name = ..., shortDescription = "", help = "")` annotation with Java's defaults.
fn annotation(
    name: &'static str,
    options: fn() -> Vec<OptionsClassRef>,
    inherits_options_from: &[&'static CommandAnnotation],
) -> &'static CommandAnnotation {
    let inherits: &'static [&'static CommandAnnotation] =
        Box::leak(inherits_options_from.to_vec().into_boxed_slice());
    let rc_inherits: &'static [&'static RcCommand] = Box::leak(
        inherits
            .iter()
            .map(|a| &a.rc)
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    );
    Box::leak(Box::new(CommandAnnotation {
        rc: RcCommand {
            name,
            inherits_options_from: rc_inherits,
        },
        options,
        inherits_options_from: inherits,
        short_description: "",
        uses_configuration_options: false,
        build_phase: BuildPhase::NONE,
        hidden: false,
        allow_residue: false,
        has_sensitive_residue: false,
        binary_std_out: false,
        binary_std_err: false,
        write_command_log: true,
        help: "",
        must_run_in_workspace: true,
        completion: "",
    }))
}

type CommandBody =
    Box<dyn Fn(&mut CommandEnvironment, &OptionsParsingResult) -> BlazeCommandResult + Send + Sync>;

struct TestCommand {
    annotation: &'static CommandAnnotation,
    body: CommandBody,
}

impl BlazeCommand for TestCommand {
    fn annotation(&self) -> &'static CommandAnnotation {
        self.annotation
    }

    fn exec(
        &self,
        env: &mut CommandEnvironment,
        options: &OptionsParsingResult,
    ) -> BlazeCommandResult {
        (self.body)(env, options)
    }
}

fn command(
    annotation: &'static CommandAnnotation,
    body: impl Fn(&mut CommandEnvironment, &OptionsParsingResult) -> BlazeCommandResult
    + Send
    + Sync
    + 'static,
) -> Arc<dyn BlazeCommand> {
    Arc::new(TestCommand {
        annotation,
        body: Box::new(body),
    })
}

fn spawn_non_zero_exit() -> FailureDetail {
    FailureDetail {
        message: String::new(),
        category: Some(Category::Spawn(failure_details::Spawn {
            code: failure_details::spawn::Code::NonZeroExit as i32,
            ..Default::default()
        })),
    }
}

/// `FooCommand` (options `--success`, `--stdout`, `--stderr`) with the given annotation.
fn foo_command(annotation: &'static CommandAnnotation) -> Arc<dyn BlazeCommand> {
    command(annotation, |env, options| {
        let foo_options = options.get_options::<FooOptions>().expect("FooOptions");
        env.get_reporter_out_err().print_out(&foo_options.stdout);
        env.get_reporter_out_err().print_err(&foo_options.stderr);
        if foo_options.exit_status {
            BlazeCommandResult::success()
        } else {
            BlazeCommandResult::failure_detail(spawn_non_zero_exit())
        }
    })
}

fn foo() -> Arc<dyn BlazeCommand> {
    foo_command(annotation("foo", foo_options, &[]))
}

fn bar() -> Arc<dyn BlazeCommand> {
    command(annotation("bar", no_options, &[]), |env, _| {
        env.get_reporter_out_err().print_out("Hello, bar.\n");
        BlazeCommandResult::success()
    })
}

type BeforeCommand =
    Box<dyn Fn(&mut CommandEnvironment) -> Result<(), AbruptExitException> + Send + Sync>;

/// A module with optional hooks; `commands` are added in `serverInit` (Java:
/// `runtime.overrideCommands`).
#[derive(Default)]
struct TestModule {
    commands: Vec<Arc<dyn BlazeCommand>>,
    before_command: Option<BeforeCommand>,
    after_command: Option<Box<dyn Fn() -> Result<(), AbruptExitException> + Send + Sync>>,
    blaze_shutdown_on_crash: Option<Box<dyn Fn(&DetailedExitCode) + Send + Sync>>,
}

impl BlazeModule for TestModule {
    fn name(&self) -> &'static str {
        "TestModule"
    }

    fn server_init(
        &self,
        _startup_options: &OptionsParsingResult,
        builder: &mut ServerBuilder,
    ) -> Result<(), AbruptExitException> {
        builder.add_commands(self.commands.iter().cloned());
        Ok(())
    }

    fn before_command(&self, env: &mut CommandEnvironment) -> Result<(), AbruptExitException> {
        match &self.before_command {
            Some(f) => f(env),
            None => Ok(()),
        }
    }

    fn after_command(&self) -> Result<(), AbruptExitException> {
        match &self.after_command {
            Some(f) => f(),
            None => Ok(()),
        }
    }

    fn blaze_shutdown_on_crash(&self, exit_code: &DetailedExitCode) {
        if let Some(f) = &self.blaze_shutdown_on_crash {
            f(exit_code);
        }
    }
}

fn commands(commands: Vec<Arc<dyn BlazeCommand>>) -> Arc<dyn BlazeModule> {
    Arc::new(TestModule {
        commands,
        ..TestModule::default()
    })
}

/// The scratch directories and the runtime (`initializeRuntime`). The tests that build a runtime
/// set process-wide state, so they run one at a time.
struct Fixture {
    runtime: Arc<BlazeRuntime>,
    _dir: tempfile::TempDir,
    _serial: MutexGuard<'static, ()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // The scratch directories go away with the fixture.
        CustomExitCodePublisher::reset_abrupt_exit_status_file();
        CustomFailureDetailPublisher::reset_failure_detail_file_path();
    }
}

fn initialize_runtime(modules: Vec<Arc<dyn BlazeModule>>) -> Fixture {
    let serial = TEST_SERIAL.lock();
    crate::bugreport::install_panic_hook();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_str().unwrap().to_string();
    for d in ["install_base", "output_base", "user_root", "scratch"] {
        std::fs::create_dir_all(format!("{root}/{d}")).unwrap();
    }
    let args = vec![
        format!("--install_base={root}/install_base"),
        format!("--output_base={root}/output_base"),
        format!("--output_user_root={root}/user_root"),
        format!("--workspace_directory={root}/scratch"),
        format!("--failure_detail_out={root}/output_base/failure_detail.rawproto"),
    ];
    let runtime = match BlazeRuntime::new_runtime(modules, &args, None) {
        Ok(runtime) => runtime,
        Err(_) => panic!("newRuntime failed"),
    };
    Fixture {
        runtime,
        _dir: dir,
        _serial: serial,
    }
}

/// `dispatch.exec(args, clientDescription, outErr)`: the test overload (`ERROR_OUT`, `NORMAL`,
/// the default policy, no startup options).
fn exec(
    dispatch: &BlazeCommandDispatcher,
    args: &[&str],
    client_description: &str,
    out_err: &RecordingOutErr,
) -> BlazeCommandResult {
    dispatch
        .exec(
            InvocationPolicy::default(),
            args.iter().map(|a| a.to_string()).collect(),
            out_err.out_err().clone(),
            LockingMode::ERROR_OUT,
            UiVerbosity::NORMAL,
            client_description,
            None,
        )
        .expect("not interrupted")
}

fn dispatcher(fixture: &Fixture, server_pid: i32) -> Arc<BlazeCommandDispatcher> {
    Arc::new(BlazeCommandDispatcher::new(
        fixture.runtime.clone(),
        server_pid,
    ))
}

fn interrupted(message: &str, code: failure_details::interrupted::Code) -> DetailedExitCode {
    DetailedExitCode::of(FailureDetail {
        message: message.to_string(),
        category: Some(Category::Interrupted(failure_details::Interrupted {
            code: code as i32,
        })),
    })
}

fn command_failure(message: &str, code: failure_details::command::Code) -> DetailedExitCode {
    DetailedExitCode::of(FailureDetail {
        message: message.to_string(),
        category: Some(Category::Command(failure_details::Command {
            code: code as i32,
        })),
    })
}

/// A command that blocks until unblocked (`BlockCommand`); `started` is signalled when it runs.
struct Block {
    started: Mutex<Option<mpsc::Receiver<()>>>,
    unblock: Arc<Barrier>,
}

fn block_command() -> (Arc<dyn BlazeCommand>, Block) {
    let (started_tx, started_rx) = mpsc::channel();
    let started_tx = Mutex::new(started_tx);
    let unblock = Arc::new(Barrier::new(2));
    let waiter = unblock.clone();
    let command = command(annotation("block", no_options, &[]), move |_, _| {
        started_tx.lock().send(()).unwrap();
        waiter.wait();
        BlazeCommandResult::success()
    });
    (
        command,
        Block {
            started: Mutex::new(Some(started_rx)),
            unblock,
        },
    )
}

impl Block {
    fn await_running(&self) {
        self.started
            .lock()
            .as_ref()
            .unwrap()
            .recv_timeout(Duration::from_secs(60))
            .unwrap();
    }
}

/// Polls `out_err`'s stderr until it contains `text`.
fn await_err_containing(out_err: &RecordingOutErr, text: &str) {
    for _ in 0..600 {
        if out_err.err_as_latin1().contains(text) {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!(
        "stderr never contained {text:?}: {}",
        out_err.err_as_latin1()
    );
}

mod blaze_command_dispatcher_test {
    use super::*;

    // Java: src/test/java/com/google/devtools/build/lib/runtime/BlazeCommandDispatcherTest.java:241 BlazeCommandDispatcherTest.testOutErrorAndExitStatus
    #[test]
    fn test_out_error_and_exit_status() {
        let fixture = initialize_runtime(vec![commands(vec![foo()])]);
        let dispatch = dispatcher(&fixture, BlazeCommandDispatcher::UNKNOWN_SERVER_PID);
        let out_err = RecordingOutErr::new();
        let result = exec(
            &dispatch,
            &[
                "foo",
                "--stdout=Hello, out.",
                "--stderr=Hello, err.",
                "--success=false",
            ],
            "test",
            &out_err,
        );
        assert!(
            out_err.out_as_latin1().ends_with("Hello, out."),
            "{}",
            out_err.out_as_latin1()
        );
        assert!(
            out_err.err_as_latin1().ends_with("Hello, err."),
            "{}",
            out_err.err_as_latin1()
        );
        assert_eq!(result.get_exit_code(), ExitCode::BUILD_FAILURE);
    }

    #[test]
    #[ignore = "needs OutOfMemoryError: a Rust allocation failure aborts, there is no OOM crash \
                (CrashFailureDetails PORT)"]
    fn test_exec_reports_hard_crash_status() {}

    // Java: src/test/java/com/google/devtools/build/lib/runtime/BlazeCommandDispatcherTest.java:285 BlazeCommandDispatcherTest.crashPreventsNewCommand
    #[test]
    fn crash_prevents_new_command() {
        let (command_started_tx, command_started_rx) = mpsc::channel();
        let command_started_tx = Mutex::new(command_started_tx);
        let hanging_command = command(annotation("testcommand", no_options, &[]), move |_, _| {
            command_started_tx.lock().send(()).unwrap();
            match interruptible::sleep(Duration::from_secs(600)) {
                Err(InterruptedException) => BlazeCommandResult::detailed_exit_code(interrupted(
                    "",
                    failure_details::interrupted::Code::Interrupted,
                )),
                Ok(()) => panic!("IllegalStateException: Should have been interrupted"),
            }
        });

        let (crash_started_tx, crash_started_rx) = mpsc::channel();
        let crash_started_tx = Mutex::new(crash_started_tx);
        let wait_to_finish_on_crash = Arc::new(Barrier::new(2));
        let waiter = wait_to_finish_on_crash.clone();
        let fixture = initialize_runtime(vec![Arc::new(TestModule {
            commands: vec![hanging_command],
            blaze_shutdown_on_crash: Some(Box::new(move |_| {
                crash_started_tx.lock().send(()).unwrap();
                waiter.wait();
            })),
            ..TestModule::default()
        })]);
        let dispatch = dispatcher(&fixture, BlazeCommandDispatcher::UNKNOWN_SERVER_PID);

        let command_dispatch = dispatch.clone();
        let command_thread = std::thread::spawn(move || {
            exec(
                &command_dispatch,
                &["testcommand"],
                "clientdesc",
                &RecordingOutErr::new(),
            )
        });

        command_started_rx
            .recv_timeout(Duration::from_secs(60))
            .unwrap();

        let crash_exit_code = DetailedExitCode::of(FailureDetail {
            message: "crash oom message".to_string(),
            category: Some(Category::Crash(failure_details::Crash {
                code: failure_details::crash::Code::CrashOom as i32,
                ..Default::default()
            })),
        });
        let runtime = fixture.runtime.clone();
        let code = crash_exit_code.clone();
        let crash_thread = std::thread::spawn(move || runtime.clean_up_for_crash(&code));

        crash_started_rx
            .recv_timeout(Duration::from_secs(60))
            .unwrap();
        let direct_result = command_thread.join().unwrap();
        assert_eq!(direct_result.get_detailed_exit_code(), &crash_exit_code);

        let recording_out_err = RecordingOutErr::new();
        let error_message = "bazel is crashing: crash oom message";
        assert_eq!(
            exec(
                &dispatch,
                &["testcommand"],
                "clientdesc",
                &recording_out_err
            )
            .get_detailed_exit_code()
            .get_failure_detail(),
            Some(&FailureDetail {
                message: error_message.to_string(),
                category: Some(Category::Command(failure_details::Command {
                    code: failure_details::command::Code::PreviouslyShutdown as i32,
                })),
            })
        );
        assert!(recording_out_err.err_as_latin1().contains(error_message));

        assert!(!crash_thread.is_finished());

        wait_to_finish_on_crash.wait();

        crash_thread.join().unwrap();
    }

    /// `CommandCompleteRecordingCommand`'s `@Subscribe onCommandComplete`.
    struct CommandCompleteRecorder(Mutex<Option<mpsc::Sender<DetailedExitCode>>>);

    impl blaze_events::EventBus for CommandCompleteRecorder {
        fn post(&self, event: Arc<dyn blaze_events::Postable>) {
            let event: Arc<dyn std::any::Any + Send + Sync> = event;
            if let Ok(command_complete) = event.downcast::<crate::bugreport::CommandCompleteEvent>()
                && let Some(tx) = self.0.lock().take()
            {
                let _ = tx.send(command_complete.get_exit_code().clone());
            }
        }
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/BlazeCommandDispatcherTest.java:367 BlazeCommandDispatcherTest.testExecReportsStatus
    #[test]
    fn test_exec_reports_status() {
        let failure_detail = spawn_non_zero_exit();
        let (tx, rx) = mpsc::channel();
        let recorder = Arc::new(CommandCompleteRecorder(Mutex::new(Some(tx))));
        let crash_command = {
            let failure_detail = failure_detail.clone();
            command(
                annotation("testcommand", no_options, &[]),
                move |env, _options| {
                    env.get_event_bus().register(recorder.clone());
                    BlazeCommandResult::failure_detail(failure_detail.clone())
                },
            )
        };
        let fixture = initialize_runtime(vec![commands(vec![crash_command])]);
        let dispatch = dispatcher(&fixture, BlazeCommandDispatcher::UNKNOWN_SERVER_PID);

        let direct_result = exec(
            &dispatch,
            &["testcommand"],
            "clientdesc",
            &RecordingOutErr::new(),
        );

        let command_complete_event = rx
            .recv_timeout(Duration::from_secs(60))
            .expect("CommandCompleteEvent");
        assert_eq!(command_complete_event, DetailedExitCode::of(failure_detail));
        assert!(!direct_result.shutdown());
    }

    fn client_env_recorder() -> (
        Arc<dyn BlazeModule>,
        Arc<Mutex<Option<Vec<(String, String)>>>>,
    ) {
        let client_env = Arc::new(Mutex::new(None));
        let recorded = client_env.clone();
        let module = Arc::new(TestModule {
            commands: vec![foo()],
            before_command: Some(Box::new(move |env| {
                *recorded.lock() = Some(
                    env.get_client_env()
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                );
                Ok(())
            })),
            ..TestModule::default()
        });
        (module, client_env)
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/BlazeCommandDispatcherTest.java:387 BlazeCommandDispatcherTest.testClientEnv
    #[test]
    fn test_client_env() {
        let (module, client_env) = client_env_recorder();
        let fixture = initialize_runtime(vec![module]);
        let dispatch = dispatcher(&fixture, BlazeCommandDispatcher::UNKNOWN_SERVER_PID);
        exec(
            &dispatch,
            &[
                "foo",
                "--client_env=V1=val1",
                "--client_env=V2=",
                "--client_env=V3=val3",
            ],
            "test",
            &RecordingOutErr::new(),
        );
        let s = |k: &str, v: &str| (k.to_string(), v.to_string());
        assert_eq!(
            client_env.lock().clone().unwrap(),
            vec![s("V1", "val1"), s("V2", ""), s("V3", "val3")]
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/BlazeCommandDispatcherTest.java:396 BlazeCommandDispatcherTest.testClientEnvEmpty
    #[test]
    fn test_client_env_empty() {
        let (module, client_env) = client_env_recorder();
        let fixture = initialize_runtime(vec![module]);
        let dispatch = dispatcher(&fixture, BlazeCommandDispatcher::UNKNOWN_SERVER_PID);
        exec(&dispatch, &["foo"], "test", &RecordingOutErr::new());
        assert_eq!(client_env.lock().clone().unwrap(), vec![]);
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/BlazeCommandDispatcherTest.java:405 BlazeCommandDispatcherTest.testAfterCommandCanModifyExitStatus
    #[test]
    fn test_after_command_can_modify_exit_status() {
        let detailed_exit_code = DetailedExitCode::of(FailureDetail {
            message: "afterCommandError".to_string(),
            category: Some(Category::BuildProgress(failure_details::BuildProgress {
                code: failure_details::build_progress::Code::BesUploadLocalFileError as i32,
            })),
        });
        let error = detailed_exit_code.clone();
        let fixture = initialize_runtime(vec![Arc::new(TestModule {
            commands: vec![foo()],
            after_command: Some(Box::new(move || {
                Err(AbruptExitException::new(error.clone()))
            })),
            ..TestModule::default()
        })]);
        let dispatch = dispatcher(&fixture, BlazeCommandDispatcher::UNKNOWN_SERVER_PID);
        let out_err = RecordingOutErr::new();
        let result = exec(&dispatch, &["foo", "--success=true"], "test", &out_err);
        assert_eq!(
            result.get_exit_code(),
            ExitCode::TRANSIENT_BUILD_EVENT_SERVICE_UPLOAD_ERROR
        );
        assert_eq!(result.get_detailed_exit_code(), &detailed_exit_code);
        assert!(out_err.err_as_latin1().contains("afterCommandError"));
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/BlazeCommandDispatcherTest.java:424 BlazeCommandDispatcherTest.testMultipleCommands
    #[test]
    fn test_multiple_commands() {
        let fixture = initialize_runtime(vec![commands(vec![foo(), bar()])]);
        let dispatch = dispatcher(&fixture, BlazeCommandDispatcher::UNKNOWN_SERVER_PID);
        let out_err = RecordingOutErr::new();

        exec(
            &dispatch,
            &["foo", "--stdout=Hello, foo."],
            "test",
            &out_err,
        );
        assert_eq!(out_err.out_as_latin1(), "Hello, foo.");
        out_err.reset();
        exec(&dispatch, &["bar"], "test", &out_err);
        assert_eq!(out_err.out_as_latin1(), "Hello, bar.\n");
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/BlazeCommandDispatcherTest.java:462 BlazeCommandDispatcherTest.testConcurrentCommandsWaitForLock
    #[test]
    fn test_concurrent_commands_wait_for_lock() {
        let (block, block_command) = block_command();
        let fixture = initialize_runtime(vec![commands(vec![bar(), block])]);
        let dispatch = dispatcher(&fixture, /* server_pid= */ 42);
        let out_err = RecordingOutErr::new();

        let block_dispatch = dispatch.clone();
        let block_command_thread = std::thread::spawn(move || {
            exec(
                &block_dispatch,
                &["block"],
                "blocking client",
                &RecordingOutErr::new(),
            )
        });
        block_command.await_running();
        let blocked_dispatch = dispatch.clone();
        let blocked_out_err = out_err.clone();
        let blocked_command_thread = std::thread::spawn(move || {
            blocked_dispatch.exec(
                InvocationPolicy::default(),
                vec!["bar".to_string()],
                blocked_out_err.out_err().clone(),
                LockingMode::WAIT,
                UiVerbosity::NORMAL,
                "test client",
                None,
            )
        });

        await_err_containing(&out_err, "Another command");
        assert!(out_err.err_as_latin1().contains(
            "Another command (blocking client) is running. Waiting for it to complete on the \
             server (server_pid=42)..."
        ));

        block_command.unblock.wait();
        block_command_thread.join().unwrap();
        let blocked = blocked_command_thread.join().unwrap().unwrap();
        // Once the lock is free, the waiting command runs.
        assert_eq!(blocked.get_exit_code(), ExitCode::SUCCESS);
        assert_eq!(out_err.out_as_latin1(), "Hello, bar.\n");
    }

    /// The `ERROR_OUT` branch of the command lock (`--noblock_for_lock`).
    // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:256 BlazeCommandDispatcher.exec
    #[test]
    fn concurrent_command_errors_out_without_waiting() {
        let (block, block_command) = block_command();
        let fixture = initialize_runtime(vec![commands(vec![bar(), block])]);
        let dispatch = dispatcher(&fixture, 42);
        let block_dispatch = dispatch.clone();
        let block_command_thread = std::thread::spawn(move || {
            exec(
                &block_dispatch,
                &["block"],
                "pid=123",
                &RecordingOutErr::new(),
            )
        });
        block_command.await_running();

        let out_err = RecordingOutErr::new();
        let result = exec(&dispatch, &["bar"], "pid=456", &out_err);
        let message = "Another command (pid=123) is running. Exiting immediately.";
        assert_eq!(out_err.err_as_latin1(), format!("{message}\n"));
        assert_eq!(
            result.get_detailed_exit_code(),
            &command_failure(
                message,
                failure_details::command::Code::AnotherCommandRunning
            )
        );
        assert_eq!(result.get_exit_code().get_numeric_exit_code(), 9);

        block_command.unblock.wait();
        block_command_thread.join().unwrap();
    }

    /// A command waiting for the lock ends with `InterruptedException` when its own thread is
    /// interrupted (a `Cancel` of that command), without touching the running command, and the
    /// lock serves the next command.
    // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:253 BlazeCommandDispatcher.exec
    #[test]
    fn waiting_for_the_lock_is_interruptible() {
        let (block, block_command) = block_command();
        let fixture = initialize_runtime(vec![commands(vec![bar(), block])]);
        let dispatch = dispatcher(&fixture, 42);
        let block_dispatch = dispatch.clone();
        let block_command_thread = std::thread::spawn(move || {
            exec(
                &block_dispatch,
                &["block"],
                "pid=123",
                &RecordingOutErr::new(),
            )
        });
        block_command.await_running();

        let out_err = RecordingOutErr::new();
        let (thread_tx, thread_rx) = mpsc::channel();
        let waiting_dispatch = dispatch.clone();
        let waiting_out_err = out_err.clone();
        let waiting_thread = std::thread::spawn(move || {
            thread_tx.send(ThreadInterrupt::current()).unwrap();
            waiting_dispatch.exec(
                InvocationPolicy::default(),
                vec!["bar".to_string()],
                waiting_out_err.out_err().clone(),
                LockingMode::WAIT,
                UiVerbosity::NORMAL,
                "pid=456",
                None,
            )
        });
        let waiting = thread_rx.recv().unwrap();
        await_err_containing(&out_err, "Another command (pid=123) is running.");
        waiting.interrupt();
        assert_eq!(
            waiting_thread.join().unwrap().err(),
            Some(InterruptedException)
        );
        assert_eq!(out_err.out_as_latin1(), "");

        block_command.unblock.wait();
        assert_eq!(
            block_command_thread.join().unwrap().get_exit_code(),
            ExitCode::SUCCESS
        );
        let next = RecordingOutErr::new();
        assert_eq!(
            exec(&dispatch, &["bar"], "pid=789", &next).get_exit_code(),
            ExitCode::SUCCESS
        );
        assert_eq!(next.out_as_latin1(), "Hello, bar.\n");
    }

    /// A panic in a command is a crash (`execExclusively`'s `catch (Throwable)`): the command
    /// ends with the crash's exit code and the shutdown flag, the lock is released, and a later
    /// command gets the shutdown reason.
    // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:862 BlazeCommandDispatcher.execExclusively
    #[test]
    fn crash_in_command_releases_the_lock() {
        let crash = command(annotation("crash", no_options, &[]), |_, _| {
            panic!("IllegalStateException: test crash")
        });
        let fixture = initialize_runtime(vec![commands(vec![crash, bar()])]);
        let dispatch = dispatcher(&fixture, 42);
        let result = exec(&dispatch, &["crash"], "pid=1", &RecordingOutErr::new());
        assert_eq!(result.get_exit_code(), ExitCode::BLAZE_INTERNAL_ERROR);
        assert!(result.shutdown());

        let out_err = RecordingOutErr::new();
        let next = exec(&dispatch, &["bar"], "pid=2", &out_err);
        let reason = "bazel is crashing: Crashed: (panic) IllegalStateException: test crash";
        assert_eq!(
            next.get_detailed_exit_code(),
            &command_failure(reason, failure_details::command::Code::PreviouslyShutdown)
        );
        assert_eq!(out_err.err_as_latin1(), format!("{reason}\n"));
    }

    /// After a command that asks for shutdown, every later command of the server gets the
    /// shutdown reason.
    // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:322 BlazeCommandDispatcher.exec
    #[test]
    fn shutdown_reason_after_explicit_shutdown() {
        let shutdown = command(annotation("shutdown", no_options, &[]), |_, _| {
            BlazeCommandResult::shutdown_on_success()
        });
        let fixture = initialize_runtime(vec![commands(vec![shutdown, bar()])]);
        let dispatch = dispatcher(&fixture, 42);
        let result = exec(&dispatch, &["shutdown"], "pid=1", &RecordingOutErr::new());
        assert_eq!(result.get_exit_code(), ExitCode::SUCCESS);
        assert!(result.shutdown());
        let out_err = RecordingOutErr::new();
        let next = exec(&dispatch, &["bar"], "pid=2", &out_err);
        let reason = "Server shut down explicitly by client pid=1";
        assert_eq!(
            next.get_detailed_exit_code(),
            &command_failure(reason, failure_details::command::Code::PreviouslyShutdown)
        );
        assert_eq!(out_err.err_as_latin1(), format!("{reason}\n"));
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/BlazeCommandDispatcherTest.java:508 BlazeCommandDispatcherTest.testDetectsInvalidCommandLineOptions
    #[test]
    fn test_detects_invalid_command_line_options() {
        let fixture = initialize_runtime(vec![commands(vec![foo()])]);
        let dispatch = dispatcher(&fixture, BlazeCommandDispatcher::UNKNOWN_SERVER_PID);
        let out_err = RecordingOutErr::new();
        let result = exec(&dispatch, &["foo", "--invalid"], "test", &out_err);
        assert_eq!(result.get_exit_code(), ExitCode::COMMAND_LINE_ERROR);
        assert!(
            out_err
                .err_as_latin1()
                .contains("Unrecognized option: --invalid\n"),
            "{}",
            out_err.err_as_latin1()
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/BlazeCommandDispatcherTest.java:517 BlazeCommandDispatcherTest.testReportsCommandNotFound
    #[test]
    fn test_reports_command_not_found() {
        let fixture = initialize_runtime(vec![commands(vec![foo()])]);
        let dispatch = dispatcher(&fixture, BlazeCommandDispatcher::UNKNOWN_SERVER_PID);
        let out_err = RecordingOutErr::new();
        let result = exec(&dispatch, &["baz"], "test", &out_err);
        assert_eq!(result.get_exit_code(), ExitCode::COMMAND_LINE_ERROR);
        assert_eq!(
            out_err.err_as_latin1(),
            "Command 'baz' not found. Try 'bazel help'.\n"
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/BlazeCommandDispatcherTest.java:527 BlazeCommandDispatcherTest.testProvidesHelpWhenNoCommandSpecified
    #[test]
    fn test_provides_help_when_no_command_specified() {
        let help = command(annotation("help", no_options, &[]), |env, _| {
            env.get_reporter_out_err()
                .print_out_ln("This is the help message.");
            BlazeCommandResult::success()
        });
        let fixture = initialize_runtime(vec![commands(vec![help])]);
        let dispatch = dispatcher(&fixture, BlazeCommandDispatcher::UNKNOWN_SERVER_PID);
        let out_err = RecordingOutErr::new();
        let result = exec(&dispatch, &[], "test", &out_err);
        assert_eq!(result.get_exit_code(), ExitCode::SUCCESS);
        assert_eq!(out_err.out_as_latin1(), "This is the help message.\n");
    }

    const BLAZERC_OPTS: &[&str] = &[
        "--rc_source=/home/jrluser/.blazerc",
        "--default_override=0:foo=--stdout",
        "--default_override=0:foo=stdout",
        "--default_override=0:foo=--stderr",
        "--default_override=0:foo=stderr",
        "--announce_rc",
    ];

    /// Java's `containsMatch` of `<a>\s+<b>\s+<c>`.
    fn contains_in_order_separated_by_whitespace(text: &str, parts: &[&str]) -> bool {
        // `\s+` backtracks: any non-empty run of whitespace that leaves the next part.
        fn matches_from(rest: &str, parts: &[&str]) -> bool {
            let Some((first, others)) = parts.split_first() else {
                return true;
            };
            let Some(after) = rest.strip_prefix(first) else {
                return false;
            };
            if others.is_empty() {
                return true;
            }
            let whitespace = after.len() - after.trim_start().len();
            (1..=whitespace).any(|k| {
                after.is_char_boundary(k) && {
                    let next = &after[k..];
                    matches_from(next, others)
                }
            })
        }
        text.char_indices()
            .any(|(i, _)| matches_from(&text[i..], parts))
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/BlazeCommandDispatcherTest.java:545 BlazeCommandDispatcherTest.testOptionsDefaults
    #[test]
    fn test_options_defaults() {
        let fixture = initialize_runtime(vec![commands(vec![foo()])]);
        let dispatch = dispatcher(&fixture, BlazeCommandDispatcher::UNKNOWN_SERVER_PID);
        let out_err = RecordingOutErr::new();

        let mut cmd_line = vec!["foo"];
        cmd_line.extend_from_slice(BLAZERC_OPTS);
        let result = exec(&dispatch, &cmd_line, "test", &out_err);
        assert_eq!(out_err.out_as_latin1(), "stdout");
        assert!(
            contains_in_order_separated_by_whitespace(
                &out_err.err_as_latin1(),
                &[
                    "INFO: Reading rc options for 'foo' from /home/jrluser/.blazerc:",
                    "  'foo' options: --stdout stdout --stderr stderr",
                    "stderr",
                ]
            ),
            "{}",
            out_err.err_as_latin1()
        );
        assert_eq!(result.get_exit_code(), ExitCode::SUCCESS);

        // Explicit options override those from config file:
        let result = exec(&dispatch, &["foo", "--success=false"], "test", &out_err);
        assert_eq!(result.get_exit_code(), ExitCode::BUILD_FAILURE);
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/BlazeCommandDispatcherTest.java:576 BlazeCommandDispatcherTest.testIllegalOptions
    #[test]
    fn test_illegal_options() {
        let fixture = initialize_runtime(vec![commands(vec![foo()])]);
        let dispatch = dispatcher(&fixture, BlazeCommandDispatcher::UNKNOWN_SERVER_PID);
        let result = exec(
            &dispatch,
            &["foo", "--not_a_valid_option"],
            "test",
            &RecordingOutErr::new(),
        );
        assert_eq!(result.get_exit_code(), ExitCode::COMMAND_LINE_ERROR);
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/BlazeCommandDispatcherTest.java:592 BlazeCommandDispatcherTest.testInheritanceOfOptionDefaults
    #[test]
    fn test_inheritance_of_option_defaults() {
        // "foo" options in ~/.blazerc should apply to "wiz" too...
        let foo_annotation = annotation("foo", foo_options, &[]);
        let wiz_annotation = annotation("wiz", no_options, &[foo_annotation]);
        let fixture = initialize_runtime(vec![commands(vec![
            foo_command(foo_annotation),
            foo_command(wiz_annotation),
        ])]);
        let dispatch = dispatcher(&fixture, BlazeCommandDispatcher::UNKNOWN_SERVER_PID);
        let out_err = RecordingOutErr::new();

        let mut cmd_line = vec!["wiz"];
        cmd_line.extend_from_slice(BLAZERC_OPTS);
        exec(&dispatch, &cmd_line, "test", &out_err);
        assert_eq!(out_err.out_as_latin1(), "stdout");
        assert!(
            contains_in_order_separated_by_whitespace(
                &out_err.err_as_latin1(),
                &[
                    "INFO: Reading rc options for 'wiz' from /home/jrluser/.blazerc:",
                    "  Inherited 'foo' options: --stdout stdout --stderr stderr",
                    "stderr",
                ]
            ),
            "{}",
            out_err.err_as_latin1()
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/BlazeCommandDispatcherTest.java:209 AnsiTestingCommand.ANSI_CODE
    const ANSI_CODE: &str = "\u{1b}[34mFoo";

    /// `AnsiTestingCommand` with the given annotation: switches the reporter to its ANSI-allowing
    /// handler and reports a STDOUT event with an ANSI code.
    ///
    /// PORT: the dispatcher does not register an ANSI-allowing handler (see the PORT note in
    /// `BlazeCommandDispatcher::exec_exclusively`), so the switch keeps the UI handler; Java's
    /// ANSI-allowing handler writes to `colorfulOutErr`, which is stripped exactly like the UI
    /// handler's streams, so what these tests assert is the same.
    // Java: src/test/java/com/google/devtools/build/lib/runtime/BlazeCommandDispatcherTest.java:208 BlazeCommandDispatcherTest.AnsiTestingCommand
    fn ansi_testing_command(annotation: &'static CommandAnnotation) -> Arc<dyn BlazeCommand> {
        command(annotation, |env, _options| {
            use blaze_events::EventHandler as _;
            let out_err = env.get_reporter().get_out_err().clone();
            env.get_reporter().switch_to_ansi_allowing_handler();
            env.get_reporter().handle(&blaze_events::Event::of_bytes(
                blaze_events::EventKind::Stdout,
                ANSI_CODE.as_bytes(),
            ));
            if out_err.get_output_stream().flush().is_err()
                || out_err.get_error_stream().flush().is_err()
            {
                return BlazeCommandResult::failure_detail(FailureDetail {
                    message: String::new(),
                    category: Some(Category::Crash(failure_details::Crash::default())),
                });
            }
            BlazeCommandResult::success()
        })
    }

    /// `@Command(name = "binary", binaryStdOut = true, shortDescription = "", help = "")`.
    // Java: src/test/java/com/google/devtools/build/lib/runtime/BlazeCommandDispatcherTest.java:230 BlazeCommandDispatcherTest.BinaryCommand
    fn binary_annotation() -> &'static CommandAnnotation {
        Box::leak(Box::new(CommandAnnotation {
            rc: RcCommand {
                name: "binary",
                inherits_options_from: &[],
            },
            options: no_options,
            inherits_options_from: &[],
            short_description: "",
            uses_configuration_options: false,
            build_phase: BuildPhase::NONE,
            hidden: false,
            allow_residue: false,
            has_sensitive_residue: false,
            binary_std_out: true,
            binary_std_err: false,
            write_command_log: true,
            help: "",
            must_run_in_workspace: true,
            completion: "",
        }))
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/BlazeCommandDispatcherTest.java:618 BlazeCommandDispatcherTest.testBinaryCommandOutput
    #[test]
    fn test_binary_command_output() {
        let binary_command = ansi_testing_command(binary_annotation());
        let fixture = initialize_runtime(vec![commands(vec![binary_command])]);
        let dispatch = dispatcher(&fixture, BlazeCommandDispatcher::UNKNOWN_SERVER_PID);
        let out_err = RecordingOutErr::new();

        // Binary commands do not remove ANSI control codes.
        let result = exec(&dispatch, &["binary", "--color=no"], "test", &out_err);
        let out = out_err.out_as_latin1();
        let err = out_err.err_as_latin1();

        assert_eq!(
            result.get_exit_code().get_numeric_exit_code(),
            ExitCode::SUCCESS.get_numeric_exit_code(),
            "{out}{err}"
        );
        assert!(out.contains(ANSI_CODE), "{out:?}");
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/BlazeCommandDispatcherTest.java:636 BlazeCommandDispatcherTest.testAsciiCommandOutput
    #[test]
    fn test_ascii_command_output() {
        let ascii_command = ansi_testing_command(annotation("ascii", no_options, &[]));
        let fixture = initialize_runtime(vec![commands(vec![ascii_command])]);
        let dispatch = dispatcher(&fixture, BlazeCommandDispatcher::UNKNOWN_SERVER_PID);
        let out_err = RecordingOutErr::new();

        // ASCII commands remove ANSI control codes.
        let result = exec(&dispatch, &["ascii", "--color=no"], "test", &out_err);
        let out = out_err.out_as_latin1();
        let err = out_err.err_as_latin1();

        assert_eq!(
            result.get_exit_code().get_numeric_exit_code(),
            ExitCode::SUCCESS.get_numeric_exit_code(),
            "{out}{err}"
        );
        assert!(!out.contains(ANSI_CODE), "{out:?}");
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/BlazeCommandDispatcherTest.java:654 BlazeCommandDispatcherTest.testWaitingForTimestampGranularityMonitor
    #[test]
    fn test_waiting_for_timestamp_granularity_monitor() {
        let fixture = initialize_runtime(vec![commands(vec![foo()])]);
        let dispatch = dispatcher(&fixture, BlazeCommandDispatcher::UNKNOWN_SERVER_PID);
        let out_err = RecordingOutErr::new();

        for _ in 0..3 {
            let result = exec(&dispatch, &["foo"], "test", &out_err);
            assert_eq!(result.get_exit_code(), ExitCode::SUCCESS);
        }

        assert_eq!(out_err.out_as_latin1(), "");
        // Java: every line matches "^|Blaze waited .* to avoid potential file system timestamp
        // granularity issues", which the empty prefix of every line does.
    }

    /// Regression test for b/136003907.
    ///
    /// Tests that even if `System.out` or `System.err` are read and retained during the lifetime
    /// of a command (which we cannot prevent, since they are public), there is no memory leak of
    /// `CommandEnvironment.getReporter`.
    ///
    /// PORT: `GcFinalization.awaitClear(weakReference)` is `Weak::upgrade` returning `None` once
    /// the command is over (reference counting frees at once).
    // Java: src/test/java/com/google/devtools/build/lib/runtime/BlazeCommandDispatcherTest.java:679 BlazeCommandDispatcherTest.noMemoryLeakOfReporterThroughSystemOutErr
    #[test]
    fn no_memory_leak_of_reporter_through_system_out_err() {
        type Retained = (
            Arc<dyn blaze_events::OutputStream>,
            Arc<dyn blaze_events::OutputStream>,
            std::sync::Weak<blaze_events::Reporter>,
        );
        let retained: Arc<Mutex<Option<Retained>>> = Arc::new(Mutex::new(None));
        let default_stdout = blaze_events::io::system_out();
        let default_stderr = blaze_events::io::system_err();
        let cmd = {
            let retained = retained.clone();
            command(
                annotation("retain_out_err", no_options, &[]),
                move |env, _options| {
                    let overridden_stdout = blaze_events::io::system_out();
                    assert!(!Arc::ptr_eq(&overridden_stdout, &default_stdout));
                    let overridden_stderr = blaze_events::io::system_err();
                    assert!(!Arc::ptr_eq(&overridden_stderr, &default_stderr));
                    let reporter = env.get_reporter();
                    *retained.lock() = Some((
                        overridden_stdout,
                        overridden_stderr,
                        Arc::downgrade(reporter),
                    ));
                    BlazeCommandResult::success()
                },
            )
        };
        let fixture = initialize_runtime(vec![commands(vec![cmd])]);
        let dispatcher = dispatcher(&fixture, BlazeCommandDispatcher::UNKNOWN_SERVER_PID);

        exec(
            &dispatcher,
            &["retain_out_err"],
            "test",
            &RecordingOutErr::new(),
        );

        // The command's System.out and System.err are still held here.
        let (_out, _err, reporter_ref) = retained.lock().take().expect("the command ran");
        assert!(
            reporter_ref.upgrade().is_none(),
            "the reporter is kept alive through the retained System.out/err"
        );
    }

    /// A subscriber that holds the command's event bus, as `ExecutionProgressReceiverSetup`
    /// does: after the command, neither the bus nor the subscriber is alive.
    ///
    /// PORT: Blaze only (review 2026-11-25-server F2). Java's collector frees the cycle; here
    /// `afterCommand` drops the bus's subscribers (`BlazeWorkspace.clearEventBus`,
    /// `Reporter.clearEventBus`).
    #[test]
    fn no_memory_leak_of_event_bus_through_its_subscribers() {
        struct HoldsBus {
            _bus: Arc<dyn blaze_events::EventBus>,
        }
        impl blaze_events::EventBus for HoldsBus {
            fn post(&self, _event: Arc<dyn blaze_events::Postable>) {}
        }
        type Retained = (
            std::sync::Weak<crate::command_environment::CommandEventBus>,
            std::sync::Weak<HoldsBus>,
        );
        let retained: Arc<Mutex<Option<Retained>>> = Arc::new(Mutex::new(None));
        let cmd = {
            let retained = retained.clone();
            command(
                annotation("register_subscriber", no_options, &[]),
                move |env, _options| {
                    let bus = env.get_event_bus().clone();
                    let subscriber = Arc::new(HoldsBus { _bus: bus.clone() });
                    bus.register(subscriber.clone());
                    *retained.lock() = Some((Arc::downgrade(&bus), Arc::downgrade(&subscriber)));
                    BlazeCommandResult::success()
                },
            )
        };
        let fixture = initialize_runtime(vec![commands(vec![cmd])]);
        let dispatcher = dispatcher(&fixture, BlazeCommandDispatcher::UNKNOWN_SERVER_PID);

        let result = exec(
            &dispatcher,
            &["register_subscriber"],
            "test",
            &RecordingOutErr::new(),
        );
        assert!(result.is_success());

        let (bus, subscriber) = retained.lock().take().expect("the command ran");
        assert!(
            subscriber.upgrade().is_none(),
            "the subscriber is kept alive by the command's event bus"
        );
        assert!(
            bus.upgrade().is_none(),
            "the command's event bus is kept alive by its subscriber"
        );
    }

    /// An early exit returns the early result; `afterCommand` still runs and its result is
    /// dropped (`BlazeCommandDispatcher.execExclusively`'s `finally`, which only logs it).
    #[test]
    fn early_exit_keeps_its_result_over_after_command() {
        let before = command_failure(
            "beforeCommandError",
            failure_details::command::Code::InvocationPolicyInvalid,
        );
        let after = DetailedExitCode::of(FailureDetail {
            message: "afterCommandError".to_string(),
            category: Some(Category::BuildProgress(failure_details::BuildProgress {
                code: failure_details::build_progress::Code::BesUploadLocalFileError as i32,
            })),
        });
        let after_called = Arc::new(AtomicBool::new(false));
        let fixture = initialize_runtime(vec![Arc::new(TestModule {
            commands: vec![foo()],
            before_command: Some({
                let before = before.clone();
                Box::new(move |_| Err(AbruptExitException::new(before.clone())))
            }),
            after_command: Some({
                let after = after.clone();
                let after_called = after_called.clone();
                Box::new(move || {
                    after_called.store(true, Ordering::SeqCst);
                    Err(AbruptExitException::new(after.clone()))
                })
            }),
            ..TestModule::default()
        })]);
        let dispatch = dispatcher(&fixture, BlazeCommandDispatcher::UNKNOWN_SERVER_PID);
        let out_err = RecordingOutErr::new();
        let result = exec(&dispatch, &["foo"], "test", &out_err);
        assert!(after_called.load(Ordering::SeqCst));
        assert_eq!(result.get_detailed_exit_code(), &before);
    }

    fn failing_before_command(code: DetailedExitCode) -> Arc<dyn BlazeModule> {
        Arc::new(TestModule {
            before_command: Some(Box::new(move |_| {
                Err(AbruptExitException::new(code.clone()))
            })),
            ..TestModule::default()
        })
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/BlazeCommandDispatcherTest.java:714 BlazeCommandDispatcherTest.useHighestPriorityExitCode
    #[test]
    fn use_highest_priority_exit_code() {
        let arbitrary_error1 = command_failure(
            "This error message should be overwritten.",
            failure_details::command::Code::CommandNotFound,
        );
        let infrastructure_failure = command_failure(
            "This is an infrastructure failure so this error should take priority.",
            failure_details::command::Code::StarlarkCpuProfileFileInitializationFailure,
        );
        let arbitrary_error2 = command_failure(
            "This error message should be overrwritten.",
            failure_details::command::Code::InvocationPolicyInvalid,
        );
        let fixture = initialize_runtime(vec![
            commands(vec![foo()]),
            failing_before_command(arbitrary_error1),
            failing_before_command(infrastructure_failure.clone()),
            failing_before_command(arbitrary_error2),
        ]);
        let dispatch = dispatcher(&fixture, BlazeCommandDispatcher::UNKNOWN_SERVER_PID);
        let out_err = RecordingOutErr::new();
        let result = exec(
            &dispatch,
            &["foo", "--config=UNDEFINED_CONFIG_VALUE"],
            "test",
            &out_err,
        );
        assert_eq!(result.get_exit_code(), ExitCode::LOCAL_ENVIRONMENTAL_ERROR);
        assert!(result.get_exit_code().is_infrastructure_failure());
        assert_eq!(result.get_detailed_exit_code(), &infrastructure_failure);
        assert!(
            out_err
                .err_as_latin1()
                .contains("This is an infrastructure failure so this error should take priority.")
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/BlazeCommandDispatcherTest.java:772 BlazeCommandDispatcherTest.useFirstExitCodeIfAllHaveEquivalentPriority
    #[test]
    fn use_first_exit_code_if_all_have_equivalent_priority() {
        // The options parsing failure should be encountered first.
        let options_parse_failure = command_failure(
            "Error parsing options: Config value 'UNDEFINED_CONFIG_VALUE' is not defined in any \
             .rc file",
            failure_details::command::Code::OptionsParseFailure,
        );
        let arbitrary_error1 = command_failure(
            "This error message should be overrwritten.",
            failure_details::command::Code::InvocationPolicyInvalid,
        );
        let arbitrary_error2 = command_failure(
            "This error message should be overwritten.",
            failure_details::command::Code::CommandNotFound,
        );
        let fixture = initialize_runtime(vec![
            commands(vec![foo()]),
            failing_before_command(arbitrary_error1),
            failing_before_command(arbitrary_error2),
        ]);
        let dispatch = dispatcher(&fixture, BlazeCommandDispatcher::UNKNOWN_SERVER_PID);
        let out_err = RecordingOutErr::new();
        let result = exec(
            &dispatch,
            &["foo", "--config=UNDEFINED_CONFIG_VALUE"],
            "test",
            &out_err,
        );
        assert_eq!(result.get_exit_code(), ExitCode::COMMAND_LINE_ERROR);
        assert_eq!(result.get_detailed_exit_code(), &options_parse_failure);
        assert!(
            out_err
                .err_as_latin1()
                .contains("Config value 'UNDEFINED_CONFIG_VALUE' is not defined in any .rc file")
        );
    }
}

mod command_interruption_test {
    use super::*;

    // Java: src/test/java/com/google/devtools/build/lib/runtime/CommandInterruptionTest.java:72 CommandInterruptionTest.UNEXPECTED_INTERRUPTION
    fn unexpected_interruption() -> DetailedExitCode {
        interrupted(
            "unexpected interruption",
            failure_details::interrupted::Code::InterruptedUnknown,
        )
    }

    /// `CommandState.SENTINEL`: "go to the barrier".
    fn sentinel() -> DetailedExitCode {
        DetailedExitCode::of(FailureDetail {
            message: "GO TO THE BARRIER".to_string(),
            category: Some(Category::Crash(failure_details::Crash {
                code: failure_details::crash::Code::CrashUnknown as i32,
                ..Default::default()
            })),
        })
    }

    /// A remote control allowing the test to control and assert on the snooze command
    /// (`CommandState`).
    struct CommandState {
        thread: ThreadInterrupt,
        /// `commandEnvironment.getBlazeModuleEnvironment()`.
        module_environment: crate::command_environment::BlazeModuleEnvironment,
        detailed_code_queue: Arc<Monitor<VecDeque<DetailedExitCode>>>,
        barrier: Arc<Barrier>,
        /// The dispatcher's result (`RunCommandThroughDispatcher`), set by `run_in`.
        result: Mutex<Option<mpsc::Receiver<DetailedExitCode>>>,
        finished: Arc<AtomicBool>,
    }

    impl CommandState {
        /// Gets the ModuleEnvironment modules will see when executing this command.
        fn get_module_environment(&self) -> &crate::command_environment::BlazeModuleEnvironment {
            &self.module_environment
        }

        fn assert_on_same_thread_as(&self, other: &CommandState) {
            assert!(self.thread.same_thread(&other.thread));
        }

        /// Sends an exit code to the command, which will then return with it if it is still
        /// running.
        fn request_exit_with(&self, detailed_exit_code: DetailedExitCode) {
            let mut queue = self.detailed_code_queue.lock();
            queue.push_back(detailed_exit_code);
            self.detailed_code_queue.notify_all();
        }

        /// Sends an interrupt directly to the command's thread.
        fn interrupt(&self) {
            self.thread.interrupt();
        }

        fn assert_not_finished_yet(&self) {
            self.request_exit_with(sentinel());
            self.barrier.wait();
            assert!(!self.finished.load(Ordering::SeqCst));
        }

        fn assert_finished_with(&self, detailed_exit_code: DetailedExitCode) {
            let result = self
                .result
                .lock()
                .as_ref()
                .expect("run_in sets the result")
                .recv_timeout(Duration::from_secs(60))
                .expect("the command finished");
            assert_eq!(result, detailed_exit_code);
        }
    }

    /// The `snooze` command: waits for an exit code from the test, either INTERRUPTED via
    /// thread interruption, or a test-specified exit code via `request_exit_with`. If
    /// `--expect_interruption` was set, a single interruption will be ignored.
    fn snooze(handoff: Arc<Mutex<Option<mpsc::Sender<CommandState>>>>) -> Arc<dyn BlazeCommand> {
        command(
            annotation("snooze", wait_options, &[]),
            move |env, options| {
                let mut expect_interruption = options
                    .get_options::<WaitOptions>()
                    .expect("WaitOptions")
                    .expect_interruption;
                let queue = Arc::new(Monitor::new(VecDeque::new()));
                let barrier = Arc::new(Barrier::new(2));
                let finished = Arc::new(AtomicBool::new(false));
                let state = CommandState {
                    thread: ThreadInterrupt::current(),
                    module_environment: env.get_blaze_module_environment().clone(),
                    detailed_code_queue: queue.clone(),
                    barrier: barrier.clone(),
                    result: Mutex::new(None),
                    finished: finished.clone(),
                };
                handoff
                    .lock()
                    .take()
                    .expect("a command is starting")
                    .send(state)
                    .unwrap();
                // waitForDetailedCodeFromTest
                let code = loop {
                    let mut detailed_code = None;
                    let take = (|| {
                        let mut q = queue.lock();
                        while q.is_empty() {
                            queue.wait(&mut q, None)?;
                        }
                        detailed_code = q.pop_front();
                        drop(q);
                        if blaze_shell::thread_interrupt::interrupted() {
                            // the interruption and the exit code delivery may have come in
                            // simultaneously.
                            return Err(InterruptedException);
                        }
                        Ok(())
                    })();
                    if take.is_err() {
                        if !expect_interruption {
                            break unexpected_interruption();
                        }
                        // That was an expected interruption; we only expect one.
                        expect_interruption = false;
                    }
                    match detailed_code {
                        Some(c) if c == sentinel() => {
                            barrier.wait();
                        }
                        Some(c) => break c,
                        None => {}
                    }
                };
                finished.store(true, Ordering::SeqCst);
                BlazeCommandResult::detailed_exit_code(code)
            },
        )
    }

    struct Setup {
        dispatcher: Arc<BlazeCommandDispatcher>,
        handoff: Arc<Mutex<Option<mpsc::Sender<CommandState>>>>,
        _fixture: Fixture,
    }

    fn set_up() -> Setup {
        let handoff = Arc::new(Mutex::new(None));
        let fixture = initialize_runtime(vec![commands(vec![snooze(handoff.clone())])]);
        Setup {
            dispatcher: dispatcher(&fixture, BlazeCommandDispatcher::UNKNOWN_SERVER_PID),
            handoff,
            _fixture: fixture,
        }
    }

    /// A single-threaded executor (Java: `Executors.newSingleThreadExecutor()`): every command
    /// submitted runs on the same thread, one after the other.
    struct Executor(Mutex<Option<mpsc::Sender<Box<dyn FnOnce() + Send>>>>);

    impl Executor {
        fn new() -> Self {
            let (tx, rx) = mpsc::channel::<Box<dyn FnOnce() + Send>>();
            std::thread::spawn(move || {
                for task in rx {
                    task();
                }
            });
            Executor(Mutex::new(Some(tx)))
        }

        fn submit(&self, task: Box<dyn FnOnce() + Send>) {
            self.0.lock().as_ref().unwrap().send(task).unwrap();
        }
    }

    /// Runs the snooze command on its own thread, waits for it to start and returns its state.
    fn run_in(setup: &Setup, expect_interruption: bool) -> CommandState {
        run_on(None, setup, expect_interruption)
    }

    /// Runs an instance of the snooze command on the given executor (a new thread without one),
    /// waits for it to start and returns a CommandState which can be used to control and assert
    /// on the status of that command (`WaitForCompletionCommand.runIn`). The state's result is
    /// the dispatcher's (`RunCommandThroughDispatcher`).
    fn run_on(
        executor: Option<&Executor>,
        setup: &Setup,
        expect_interruption: bool,
    ) -> CommandState {
        let (tx, rx) = mpsc::channel();
        *setup.handoff.lock() = Some(tx);
        let dispatcher = setup.dispatcher.clone();
        let (result_tx, result_rx) = mpsc::channel();
        let task: Box<dyn FnOnce() + Send> = Box::new(move || {
            let result = exec(
                &dispatcher,
                &[
                    "snooze",
                    if expect_interruption {
                        "--expect_interruption"
                    } else {
                        "--noexpect_interruption"
                    },
                ],
                "CommandInterruptionTest",
                &RecordingOutErr::new(),
            );
            let _ = result_tx.send(result.get_detailed_exit_code().clone());
        });
        match executor {
            Some(executor) => executor.submit(task),
            None => {
                std::thread::spawn(task);
            }
        }
        let state: CommandState = rx.recv_timeout(Duration::from_secs(60)).unwrap();
        *state.result.lock() = Some(result_rx);
        state
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/CommandInterruptionTest.java:67 CommandInterruptionTest.NO_TEST_TARGETS_CODE
    fn no_test_targets_code() -> DetailedExitCode {
        DetailedExitCode::of(FailureDetail {
            message: String::new(),
            category: Some(Category::TestCommand(failure_details::TestCommand {
                code: failure_details::test_command::Code::NoTestTargets as i32,
            })),
        })
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/CommandInterruptionTest.java:79 CommandInterruptionTest.CRASH
    fn crash() -> DetailedExitCode {
        DetailedExitCode::of(FailureDetail {
            message: "crash".to_string(),
            category: Some(Category::Crash(failure_details::Crash {
                code: failure_details::crash::Code::CrashUnknown as i32,
                ..Default::default()
            })),
        })
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/CommandInterruptionTest.java:84 CommandInterruptionTest.OPTIONS_FAILURE
    fn options_failure() -> DetailedExitCode {
        DetailedExitCode::of(FailureDetail {
            message: String::new(),
            category: Some(Category::Command(failure_details::Command {
                code: failure_details::command::Code::OptionsParseFailure as i32,
            })),
        })
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/CommandInterruptionTest.java:428 CommandInterruptionTest.sendingExitCodeToTestCommandResultsInExitWithThatStatus
    #[test]
    fn sending_exit_code_to_test_command_results_in_exit_with_that_status() {
        let setup = set_up();
        let command = run_in(&setup, false);
        command.request_exit_with(DetailedExitCode::success());
        command.assert_finished_with(DetailedExitCode::success());
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/CommandInterruptionTest.java:435 CommandInterruptionTest.interruptingTestCommandMakesItExitWithInterruptedStatus
    #[test]
    fn interrupting_test_command_makes_it_exit_with_interrupted_status() {
        let setup = set_up();
        let command = run_in(&setup, false);
        command.interrupt();
        command.assert_finished_with(unexpected_interruption());
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/CommandInterruptionTest.java:442 CommandInterruptionTest.commandIgnoresFirstInterruptionWhenExpectingInterruption
    #[test]
    fn command_ignores_first_interruption_when_expecting_interruption() {
        let setup = set_up();
        let command = run_in(&setup, true);
        command.interrupt();
        command.assert_not_finished_yet();
        command.request_exit_with(DetailedExitCode::success());
        command.assert_finished_with(DetailedExitCode::success());
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/CommandInterruptionTest.java:451 CommandInterruptionTest.commandExitsWithInterruptedAfterInterruptionCountExceeded
    #[test]
    fn command_exits_with_interrupted_after_interruption_count_exceeded() {
        let setup = set_up();
        let command = run_in(&setup, true);
        command.interrupt();
        command.assert_not_finished_yet();
        command.interrupt();
        command.assert_finished_with(unexpected_interruption());
    }

    #[test]
    #[ignore = "exit(null): a null exception is not representable (AbruptExitException is taken \
                by value)"]
    fn exit_forbids_null_exception() {}

    // Java: src/test/java/com/google/devtools/build/lib/runtime/CommandInterruptionTest.java:474 CommandInterruptionTest.callingExitOnceInterruptsAndOverridesExitCode
    #[test]
    fn calling_exit_once_interrupts_and_overrides_exit_code() {
        let setup = set_up();
        let command = run_in(&setup, false);
        command
            .get_module_environment()
            .exit(AbruptExitException::new(no_test_targets_code()));
        command.assert_finished_with(no_test_targets_code());
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/CommandInterruptionTest.java:481 CommandInterruptionTest.callingExitSecondTimeNeitherInterruptsNorReOverridesExitCode
    #[test]
    fn calling_exit_second_time_neither_interrupts_nor_re_overrides_exit_code() {
        let setup = set_up();
        let command = run_in(&setup, true);
        command
            .get_module_environment()
            .exit(AbruptExitException::new(no_test_targets_code()));
        command.assert_not_finished_yet();
        command
            .get_module_environment()
            .exit(AbruptExitException::new(options_failure()));
        command.assert_not_finished_yet();
        command.request_exit_with(DetailedExitCode::success());
        command.assert_finished_with(no_test_targets_code());
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/CommandInterruptionTest.java:492 CommandInterruptionTest.abruptExitCodesDontOverrideInfrastructureFailures
    #[test]
    fn abrupt_exit_codes_dont_override_infrastructure_failures() {
        let setup = set_up();
        let command = run_in(&setup, true);
        command
            .get_module_environment()
            .exit(AbruptExitException::new(no_test_targets_code()));
        command.assert_not_finished_yet();
        command.request_exit_with(crash());
        command.assert_finished_with(crash());
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/CommandInterruptionTest.java:501 CommandInterruptionTest.callingExitAfterCommandCompletesDoesNothing
    #[test]
    fn calling_exit_after_command_completes_does_nothing() {
        let setup = set_up();
        let executor = Executor::new();
        let first_command = run_on(Some(&executor), &setup, false);
        first_command.request_exit_with(DetailedExitCode::success());
        first_command.assert_finished_with(DetailedExitCode::success());
        let new_command_on_same_thread = run_on(Some(&executor), &setup, false);
        first_command.assert_on_same_thread_as(&new_command_on_same_thread);
        first_command
            .get_module_environment()
            .exit(AbruptExitException::new(options_failure()));
        new_command_on_same_thread.assert_not_finished_yet();
        new_command_on_same_thread.request_exit_with(DetailedExitCode::success());
        new_command_on_same_thread.assert_finished_with(DetailedExitCode::success());
    }
}

/// Several commands of the real Bazel modules through one runtime and one dispatcher, as a
/// server runs them: `version`, `info`, a genrule `build` twice, a `build` whose command thread
/// is interrupted (the server's `Cancel`), then a `build` that must not start interrupted.
///
/// Needs the oracle's install base (`embedded_tools`, `linux-sandbox`) and the shared repository
/// cache (AGENTS.md "Fixed facts"); skipped when the install base is missing.
mod server_commands {
    use super::*;

    const INSTALL_MD5: &str = "58cf1ecbff441bac762b0050d6da21e9";

    fn home() -> std::path::PathBuf {
        std::path::PathBuf::from(std::env::var_os("HOME").expect("HOME"))
    }

    fn run(
        dispatch: &BlazeCommandDispatcher,
        args: &[String],
        out_err: &RecordingOutErr,
    ) -> BlazeCommandResult {
        dispatch
            .exec(
                InvocationPolicy::default(),
                args.to_vec(),
                out_err.out_err().clone(),
                LockingMode::WAIT,
                UiVerbosity::NORMAL,
                "pid=1",
                None,
            )
            .expect("not interrupted while waiting for the lock")
    }

    #[test]
    fn several_commands_through_one_runtime_and_dispatcher() {
        let install_base = home().join(format!(".cache/blaze/oracle-out/ur/install/{INSTALL_MD5}"));
        if !install_base.join("embedded_tools").is_dir() {
            eprintln!(
                "skipping: no oracle install base at {}",
                install_base.display()
            );
            return;
        }
        let repository_cache = home().join(".cache/blaze/repo-cache");

        let serial = TEST_SERIAL.lock();
        crate::bugreport::install_panic_hook();
        // Bazel.main, once per process.
        static BUILD_INFO: std::sync::Once = std::sync::Once::new();
        BUILD_INFO.call_once(|| {
            crate::version_info::BlazeVersionInfo::set_build_info(
                crate::version_info::try_get_build_info(),
            )
        });
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap().to_string();
        let ws = format!("{root}/ws");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::write(format!("{ws}/MODULE.bazel"), "").unwrap();
        std::fs::write(
            format!("{ws}/BUILD"),
            "genrule(\n    name = \"g\",\n    outs = [\"g.txt\"],\n    cmd = \"echo hello > $@\",\n)\n\n\
             genrule(\n    name = \"slow\",\n    outs = [\"slow.txt\"],\n    cmd = \"sleep 120; echo slow > $@\",\n)\n",
        )
        .unwrap();
        let ob = format!("{root}/ob");
        std::fs::create_dir_all(&ob).unwrap();
        let args = vec![
            format!("--install_base={}", install_base.display()),
            format!("--install_md5={INSTALL_MD5}"),
            format!("--output_base={ob}"),
            format!("--output_user_root={root}"),
            format!("--workspace_directory={ws}"),
            format!("--failure_detail_out={ob}/failure_detail.rawproto"),
        ];
        let runtime =
            match BlazeRuntime::new_runtime(crate::bazel_modules::bazel_modules(), &args, None) {
                Ok(runtime) => runtime,
                Err(_) => panic!("newRuntime failed"),
            };
        let _fixture = Fixture {
            runtime: runtime.clone(),
            _dir: dir,
            _serial: serial,
        };
        let dispatch = Arc::new(BlazeCommandDispatcher::new(runtime, 4242));
        let command = |args: &[&str]| -> Vec<String> {
            let mut v: Vec<String> = args.iter().map(|a| a.to_string()).collect();
            v.push(format!("--client_cwd={ws}"));
            v
        };
        let build = |target: &str| -> Vec<String> {
            command(&[
                "build",
                target,
                &format!("--repository_cache={}", repository_cache.display()),
                "--color=no",
                "--curses=no",
            ])
        };

        let out_err = RecordingOutErr::new();
        let result = run(&dispatch, &command(&["version"]), &out_err);
        assert_eq!(
            result.get_exit_code(),
            ExitCode::SUCCESS,
            "{}",
            out_err.err_as_latin1()
        );
        assert!(out_err.out_as_latin1().starts_with("Build label:"));

        let out_err = RecordingOutErr::new();
        let result = run(&dispatch, &command(&["info", "output_base"]), &out_err);
        assert_eq!(
            result.get_exit_code(),
            ExitCode::SUCCESS,
            "{}",
            out_err.err_as_latin1()
        );
        assert_eq!(out_err.out_as_latin1(), format!("{ob}\n"));

        for _ in 0..2 {
            let out_err = RecordingOutErr::new();
            let result = run(&dispatch, &build("//:g"), &out_err);
            assert_eq!(
                result.get_exit_code(),
                ExitCode::SUCCESS,
                "{}",
                out_err.err_as_latin1()
            );
            assert!(
                out_err
                    .err_as_latin1()
                    .contains("INFO: Build completed successfully"),
                "{}",
                out_err.err_as_latin1()
            );
        }

        // A cancelled command: its thread is interrupted while the genrule sleeps.
        let out_err = RecordingOutErr::new();
        let (thread_tx, thread_rx) = mpsc::channel();
        let slow_dispatch = dispatch.clone();
        let slow_out_err = out_err.clone();
        let slow_args = build("//:slow");
        let slow = std::thread::spawn(move || {
            thread_tx.send(ThreadInterrupt::current()).unwrap();
            run(&slow_dispatch, &slow_args, &slow_out_err)
        });
        let slow_thread = thread_rx.recv().unwrap();
        // The progress line of the running genrule (Bazel prints "Found 1 target..." only once
        // the merged analysis and execution phase returned).
        await_err_containing(&out_err, "Executing genrule //:slow");
        std::thread::sleep(Duration::from_secs(2));
        slow_thread.interrupt();
        let result = slow.join().unwrap();
        assert_eq!(
            result.get_exit_code(),
            ExitCode::INTERRUPTED,
            "{}",
            out_err.err_as_latin1()
        );

        // The next command is not interrupted.
        let out_err = RecordingOutErr::new();
        let result = run(&dispatch, &build("//:g"), &out_err);
        assert_eq!(
            result.get_exit_code(),
            ExitCode::SUCCESS,
            "{}",
            out_err.err_as_latin1()
        );
        let out_err = RecordingOutErr::new();
        let result = run(&dispatch, &command(&["info", "server_pid"]), &out_err);
        assert_eq!(result.get_exit_code(), ExitCode::SUCCESS);
        // ServerPidInfoItem: ProcessHandle.current().pid().
        assert_eq!(out_err.out_as_latin1(), format!("{}\n", std::process::id()));
    }
}

/// Ports of the idle-task tests of `src/test/java/com/google/devtools/build/lib/runtime/
/// BlazeRuntimeTest.java`. Java calls `runtime.beforeCommand(env, options)` on a hand-built
/// environment; here the dispatcher builds the environment and runs `beforeCommand` with the
/// command line's options, and the tasks are read from the command's result (Java's
/// `afterCommand` hands `env.getIdleTasks()` to it).
mod blaze_runtime_idle_tasks {
    use super::*;
    use blaze_server::IdleTask;

    /// Runs `bar` with `args` and returns its result.
    fn run_bar(extra_modules: Vec<Arc<dyn BlazeModule>>, args: &[&str]) -> BlazeCommandResult {
        let mut modules = vec![commands(vec![bar()])];
        modules.extend(extra_modules);
        let fixture = initialize_runtime(modules);
        let dispatch = dispatcher(&fixture, BlazeCommandDispatcher::UNKNOWN_SERVER_PID);
        let out_err = RecordingOutErr::new();
        let mut command_line = vec!["bar"];
        command_line.extend_from_slice(args);
        let result = exec(&dispatch, &command_line, "test", &out_err);
        assert_eq!(
            result.get_exit_code(),
            ExitCode::SUCCESS,
            "{}",
            out_err.err_as_latin1()
        );
        result
    }

    fn tasks_named<'a>(result: &'a BlazeCommandResult, name: &str) -> Vec<&'a Arc<dyn IdleTask>> {
        result
            .get_idle_tasks()
            .iter()
            .filter(|t| t.display_name() == name)
            .collect()
    }

    #[test]
    fn adds_gc_and_interner_shrinking_idle_task_no_state_kept_after_build() {
        let result = run_bar(vec![], &["--keep_state_after_build=false"]);
        let gc_idle_tasks = tasks_named(&result, "GC and interner shrinking");
        assert_eq!(gc_idle_tasks.len(), 1);
        assert_eq!(gc_idle_tasks[0].delay(), Duration::ZERO);
    }

    #[test]
    fn adds_gc_and_interner_shrinking_idle_task_state_kept_after_build() {
        let result = run_bar(vec![], &["--keep_state_after_build=true"]);
        let gc_idle_tasks = tasks_named(&result, "GC and interner shrinking");
        assert_eq!(gc_idle_tasks.len(), 1);
        assert!(gc_idle_tasks[0].delay() > Duration::ZERO);
    }

    #[test]
    fn does_not_add_install_base_gc_idle_task_when_disabled() {
        let result = run_bar(vec![], &["--experimental_install_base_gc_max_age=0"]);
        assert!(tasks_named(&result, "Install base garbage collector").is_empty());
    }

    #[test]
    fn adds_install_base_gc_idle_task_when_enabled() {
        let result = run_bar(vec![], &["--experimental_install_base_gc_max_age=365d"]);
        let gc_idle_tasks = tasks_named(&result, "Install base garbage collector");
        assert_eq!(gc_idle_tasks.len(), 1);
        assert_eq!(gc_idle_tasks[0].delay(), Duration::ZERO);
        // PORT: the collector's root, own install base and max age are checked on a task built
        // the same way (`idle_tasks::tests::install_base_gc_idle_task_create`); the trait object
        // cannot be downcast.
    }

    #[test]
    fn does_not_add_action_cache_gc_idle_task_when_disabled() {
        let result = run_bar(
            vec![],
            &[
                "--experimental_action_cache_gc_max_age=0",
                "--experimental_action_cache_gc_idle_delay=5m",
                "--experimental_action_cache_gc_threshold=10",
            ],
        );
        assert!(tasks_named(&result, "Action cache garbage collector").is_empty());
    }

    #[test]
    fn adds_action_cache_gc_idle_task_when_enabled() {
        let result = run_bar(
            vec![],
            &[
                "--experimental_action_cache_gc_max_age=7d",
                "--experimental_action_cache_gc_idle_delay=5m",
                "--experimental_action_cache_gc_threshold=10",
            ],
        );
        let gc_idle_tasks = tasks_named(&result, "Action cache garbage collector");
        assert_eq!(gc_idle_tasks.len(), 1);
        assert_eq!(gc_idle_tasks[0].delay(), Duration::from_secs(5 * 60));
    }

    struct NamedTask(&'static str);

    impl IdleTask for NamedTask {
        fn display_name(&self) -> String {
            self.0.to_string()
        }

        fn run(&self) -> Result<(), blaze_server::IdleTaskError> {
            Ok(())
        }
    }

    #[test]
    fn adds_idle_tasks_from_modules() {
        let module: Arc<dyn BlazeModule> = Arc::new(TestModule {
            before_command: Some(Box::new(|env| {
                env.add_idle_task(Arc::new(NamedTask("foo")));
                env.add_idle_task(Arc::new(NamedTask("bar")));
                Ok(())
            })),
            ..TestModule::default()
        });
        let result = run_bar(vec![module], &[]);
        assert_eq!(tasks_named(&result, "foo").len(), 1);
        assert_eq!(tasks_named(&result, "bar").len(), 1);
    }
}

/// The repo contents cache's shared lock is the command's: `BazelRepositoryModule` takes it in
/// `beforeCommand` and releases it in `afterCommand`, so a server holds it only while a command
/// runs (its garbage collector, an idle task, takes the exclusive lock between commands).
mod bazel_repository_module {
    use super::*;
    use blaze_repo_fetch::file_system_lock::FileSystemLock;

    fn lock_path(cache: &std::path::Path) -> blaze_vfs::Path {
        blaze_vfs::Path::create(
            blaze_vfs::PathFragment::create(&format!("{}/gc_lock", cache.display())),
            &blaze_vfs::UnixFileSystem::create(blaze_vfs::DigestHashFunction::Sha256),
        )
    }

    // Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:407 BazelRepositoryModule.beforeCommand
    // Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:734 BazelRepositoryModule.afterCommand
    #[test]
    fn repo_contents_cache_shared_lock_is_held_for_the_command_only() {
        let cache: Arc<Mutex<Option<std::path::PathBuf>>> = Arc::new(Mutex::new(None));
        let held_during_command = Arc::new(AtomicBool::new(false));
        let probe = {
            let cache = cache.clone();
            let held_during_command = held_during_command.clone();
            command(annotation("bar", no_options, &[]), move |_env, _| {
                let path = lock_path(cache.lock().as_ref().unwrap());
                held_during_command.store(
                    FileSystemLock::held_by_this_process(&path),
                    Ordering::SeqCst,
                );
                BlazeCommandResult::success()
            })
        };
        let fixture = initialize_runtime(vec![
            Arc::new(crate::bazel_modules::BazelRepositoryModule::default()),
            commands(vec![probe]),
        ]);
        let dir = fixture._dir.path().join("cc");
        *cache.lock() = Some(dir.clone());
        let dispatch = dispatcher(&fixture, BlazeCommandDispatcher::UNKNOWN_SERVER_PID);
        for _ in 0..2 {
            let out_err = RecordingOutErr::new();
            let flag = format!("--repo_contents_cache={}", dir.display());
            let result = exec(&dispatch, &["bar", &flag], "test", &out_err);
            assert_eq!(
                result.get_exit_code(),
                ExitCode::SUCCESS,
                "{}",
                out_err.err_as_latin1()
            );
            assert!(held_during_command.swap(false, Ordering::SeqCst));
            assert!(!FileSystemLock::held_by_this_process(&lock_path(&dir)));
        }
    }
}
