//! A command server with a fake dispatcher: the fixture of `scripts/server/interop-server`.
//!
//! The real Bazel client starts it through `--server_javabase=<dir>` with `<dir>/bin/java` a
//! symlink to this binary, so it receives the argument vector the client gives the JVM:
//! `argv[0] = "bazel(<workspace>)"`, the JVM flags, `-jar <path>`, then the startup options.
//! It serves `GrpcServerImpl` exactly as `BlazeRuntime.serverMain` will, with a dispatcher whose
//! commands exercise the protocol (the command's positional arguments are those that do not
//! start with `--`; the client adds its own `--` options):
//!
//! - `echo ARG...`: for each argument, `out:ARG\n` to stdout then `err:ARG\n` to stderr; exit 0.
//! - `shutdown`: shuts the server down after the response (`shutdownOnSuccess`).
//! - `sleep N`: sleeps N seconds, interruptibly (Cancel); exit 8 when interrupted.
//! - `exit N`: exits with code N.
//! - `big N`: writes N bytes (`i % 251` for i in 0..N) to stdout, then `big done\n` to stderr.
//! - `exec`: returns an `ExecRequest` that makes the client exec `/bin/echo exec-ok`.
//! - `crash`: panics on the command thread (the uncaught-exception path; exit 37).

use std::ffi::OsString;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::sync::Arc;
use std::time::Duration;

use blaze_events::io::OutErr;
use blaze_events::ui::JavaClock;
use blaze_protos::blaze::invocation_policy::InvocationPolicy;
use blaze_protos::command_server::ExecRequest;
use blaze_protos::failure_details::failure_detail::Category;
use blaze_protos::failure_details::{FailureDetail, Interrupted, interrupted};
use blaze_protos::google::protobuf::Any;
use blaze_server::command_dispatcher::IdleTaskResultsSupplier;
use blaze_server::interruptible;
use blaze_server::shutdown_hooks::run_registered_shutdown_hooks;
use blaze_server::{
    BlazeCommandResult, CommandDispatcher, CommandExtensionReporter, GrpcServerImpl, LockingMode,
    PidFileWatcher, ShutdownHooks, UiVerbosity,
};
use blaze_shell::thread_interrupt::InterruptedException;
use blaze_util::exit_code::{DetailedExitCode, ExitCode};
use blaze_util::string_encoding::{latin1_decode, latin1_encode};
use blaze_vfs::{DigestHashFunction, Path, UnixFileSystem};

struct ProbeDispatcher;

fn write(out: &OutErr, stdout: bool, bytes: &[u8]) {
    let stream = if stdout {
        out.get_output_stream()
    } else {
        out.get_error_stream()
    };
    let _ = stream.write(bytes);
}

fn exit_with(code: i32, message: &str) -> BlazeCommandResult {
    let exit_code = ExitCode::for_code(code).unwrap_or(ExitCode::BLAZE_INTERNAL_ERROR);
    BlazeCommandResult::detailed_exit_code(DetailedExitCode::of_exit_code(
        exit_code,
        FailureDetail {
            message: message.to_string(),
            category: None,
        },
    ))
}

impl CommandDispatcher for ProbeDispatcher {
    fn exec(
        &self,
        _invocation_policy: InvocationPolicy,
        args: Vec<String>,
        out_err: OutErr,
        _locking_mode: LockingMode,
        _ui_verbosity: UiVerbosity,
        _client_description: String,
        _first_contact_time_millis: i64,
        _startup_options_tagged_with_bazel_rc: Option<Vec<(String, String)>>,
        _idle_task_results_supplier: IdleTaskResultsSupplier,
        _command_extensions: Vec<Any>,
        _command_extension_reporter: Arc<dyn CommandExtensionReporter>,
    ) -> Result<BlazeCommandResult, InterruptedException> {
        let command = args.first().cloned().unwrap_or_default();
        let positional: Vec<&String> = args
            .iter()
            .skip(1)
            .filter(|a| !a.starts_with("--"))
            .collect();
        let number = |default: u64| -> u64 {
            positional
                .first()
                .and_then(|n| n.parse().ok())
                .unwrap_or(default)
        };
        Ok(match command.as_str() {
            "echo" => {
                for arg in &positional {
                    let bytes = latin1_encode(arg);
                    write(&out_err, true, &[b"out:", &bytes[..], b"\n"].concat());
                    write(&out_err, false, &[b"err:", &bytes[..], b"\n"].concat());
                }
                BlazeCommandResult::success()
            }
            "shutdown" => BlazeCommandResult::shutdown_on_success(),
            "sleep" => {
                let seconds = number(60);
                write(&out_err, false, format!("sleeping {seconds}s\n").as_bytes());
                match interruptible::sleep(Duration::from_secs(seconds)) {
                    Ok(()) => {
                        write(&out_err, false, b"slept\n");
                        BlazeCommandResult::success()
                    }
                    Err(InterruptedException) => {
                        write(&out_err, false, b"sleep interrupted\n");
                        BlazeCommandResult::failure_detail(FailureDetail {
                            message: "sleep interrupted".to_string(),
                            category: Some(Category::Interrupted(Interrupted {
                                code: interrupted::Code::Interrupted as i32,
                            })),
                        })
                    }
                }
            }
            "exit" => {
                let code = number(0) as i32;
                exit_with(code, &format!("exit {code}"))
            }
            "big" => {
                let n = number(0) as usize;
                let data: Vec<u8> = (0..n).map(|i| (i % 251) as u8).collect();
                // Several writes of uneven size, so that chunking at 8192 bytes is exercised.
                for piece in data.chunks(100_000) {
                    write(&out_err, true, piece);
                }
                write(&out_err, false, b"big done\n");
                BlazeCommandResult::success()
            }
            "exec" => {
                let cwd = std::env::current_dir().unwrap_or_default();
                BlazeCommandResult::execute(ExecRequest {
                    working_directory: cwd.into_os_string().into_vec(),
                    argv: vec![b"/bin/echo".to_vec(), b"exec-ok".to_vec()],
                    should_exec: true,
                    ..Default::default()
                })
            }
            "crash" => panic!("probe: crash requested"),
            other => {
                write(
                    &out_err,
                    false,
                    format!("probe: unknown command '{other}'\n").as_bytes(),
                );
                exit_with(2, "unknown command")
            }
        })
    }
}

/// The startup options the probe reads (`BlazeServerStartupOptions`).
#[derive(Default, Debug)]
struct StartupOptions {
    output_base: Option<String>,
    max_idle_secs: i32,
    command_port: i32,
    shutdown_on_low_sys_mem: bool,
    idle_server_tasks: bool,
}

fn parse_startup_options(argv: &[OsString]) -> StartupOptions {
    // Skip everything up to and including `-jar <path>`.
    let start = argv
        .iter()
        .position(|a| a.as_bytes() == b"-jar")
        .map(|i| i + 2)
        .unwrap_or(1);
    let mut options = StartupOptions {
        max_idle_secs: 3 * 3600,
        idle_server_tasks: true,
        ..Default::default()
    };
    for arg in argv.iter().skip(start) {
        let arg = latin1_decode(arg.as_bytes()).into_owned();
        if let Some(v) = arg.strip_prefix("--output_base=") {
            options.output_base = Some(v.to_string());
        } else if let Some(v) = arg.strip_prefix("--max_idle_secs=") {
            options.max_idle_secs = v.parse().unwrap_or(options.max_idle_secs);
        } else if let Some(v) = arg.strip_prefix("--command_port=") {
            options.command_port = v.parse().unwrap_or(0);
        } else if arg == "--shutdown_on_low_sys_mem" {
            options.shutdown_on_low_sys_mem = true;
        } else if arg == "--noshutdown_on_low_sys_mem" {
            options.shutdown_on_low_sys_mem = false;
        } else if arg == "--idle_server_tasks" {
            options.idle_server_tasks = true;
        } else if arg == "--noidle_server_tasks" {
            options.idle_server_tasks = false;
        }
    }
    options
}

fn main() {
    let argv: Vec<OsString> = std::env::args_os().collect();
    let options = parse_startup_options(&argv);
    let Some(output_base) = options.output_base.clone() else {
        eprintln!("probe: no --output_base= in {argv:?}");
        std::process::exit(ExitCode::COMMAND_LINE_ERROR.get_numeric_exit_code());
    };
    eprintln!("probe: startup options {options:?}");
    let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
    let server_directory = Path::create_from_str(&output_base, &fs).get_child("server");

    // BlazeRuntime.serverMain: server.pid was written by the launcher (daemonize).
    let pid_file = server_directory.get_child("server.pid.txt");
    let pid_contents = std::fs::read(&*blaze_util::string_encoding::internal_to_platform(
        pid_file.get_path_string(),
    ))
    .unwrap_or_default();
    let server_pid: i32 = match latin1_decode(&pid_contents).parse() {
        Ok(pid) => pid,
        Err(_) => std::process::id() as i32,
    };
    let pid_file_watcher = Arc::new(PidFileWatcher::new(pid_file.clone(), server_pid));
    pid_file_watcher.start();

    let shutdown_hooks = ShutdownHooks::create_and_register();
    shutdown_hooks.cleanup_pid_file(pid_file, pid_file_watcher.clone());

    let rpc_server = GrpcServerImpl::create(
        Arc::new(ProbeDispatcher),
        shutdown_hooks,
        pid_file_watcher,
        Arc::new(JavaClock),
        options.command_port,
        server_directory,
        server_pid,
        options.max_idle_secs,
        options.shutdown_on_low_sys_mem,
        options.idle_server_tasks,
        None,
    );
    let code = match rpc_server.serve() {
        Ok(()) => ExitCode::SUCCESS.get_numeric_exit_code(),
        Err(e) => {
            eprintln!("{}", e.get_message());
            e.get_exit_code().get_numeric_exit_code()
        }
    };
    eprintln!("probe: serve returned, exiting with {code}");
    run_registered_shutdown_hooks();
    std::process::exit(code);
}
