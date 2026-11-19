//! The client/server seam.
//!
//! Bazel's client either `exec`s the server JVM in `--batch` mode (`RunBatchMode`) or starts a
//! long-lived server and sends it one `command_server.RunRequest` per command over gRPC
//! (`RunClientServerMode` / `BlazeServer::Communicate`). Until wave W8 Blaze runs the server
//! in-process (docs/crates.md): the client builds exactly the arguments and the request the server
//! would receive and hands them to an [`InProcessServer`] instead of a JVM.

use blaze_protos::command_server::{RunRequest, RunResponse};

/// What the client hands to the server when it would start the server JVM.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerStartup {
    /// The whole argument vector `GetServerExeArgs` builds: `bazel(<workspace>)`, the JVM flags,
    /// `-jar <server jar>`, then the startup options in `--flag=value` form. This is what the
    /// client writes to `<output_base>/server/cmdline` (NUL-joined).
    pub server_exe_args: Vec<String>,
    /// Index in `server_exe_args` of the first startup option: `server_exe_args[first_startup_arg..]`
    /// is the `String[] args` that `BlazeRuntime.main` receives.
    pub first_startup_arg: usize,
}

impl ServerStartup {
    /// The arguments `BlazeRuntime.main` receives (the startup options).
    pub fn startup_args(&self) -> &[String] {
        &self.server_exe_args[self.first_startup_arg..]
    }
}

/// The server, run in-process. Implemented by the `blaze` binary on top of `blaze_runtime`.
///
/// Output goes directly to this process's stdout and stderr (the server's `OutErr` is the
/// client's), so neither method returns output.
pub trait InProcessServer {
    /// `RunBatchMode`: the JVM would be exec'd with `startup.server_exe_args` followed by the
    /// command, the logging args (`AddLoggingArgs`) and the command arguments. `args` is that
    /// `main(String[] args)` vector without the JVM part: the startup options (starting with
    /// `--batch`), then the command, logging args and command arguments. Returns the process exit
    /// code.
    fn batch_main(&self, startup: &ServerStartup, args: Vec<String>) -> i32;

    /// `RunClientServerMode`: start a server with `startup` (`BlazeRuntime.main` in server mode)
    /// and run one `RunRequest`, built exactly as `BlazeServer::Communicate` builds it (args =
    /// command, logging args, command arguments; cookie empty). Returns the final `RunResponse`
    /// (`finished`, `exit_code`, the `run` command's `exec_request`, `failure_detail`; its
    /// output fields are empty: the server wrote to this process's stdout and stderr). The
    /// server shuts down when the command finishes.
    fn run_command(&self, startup: &ServerStartup, request: RunRequest) -> RunResponse;

    /// The server's cancel of the running command (`GrpcServerImpl.cancel` /
    /// `CommandManager.doCancel`: interrupts the command thread), which the client's cancel
    /// thread calls when the user presses Ctrl-C (`BlazeServer::SendCancelMessage`).
    fn canceller(&self) -> std::sync::Arc<dyn Fn() + Send + Sync>;
}

/// Which server the client talks to: the environment variable `BLAZE_SERVER_MODE` of the
/// client, read once by `blaze::Main`.
///
/// PORT: Blaze-only (the persistent server is opt-in until it becomes the default).
/// - unset or `inprocess`: the server runs in this process ([`InProcessServer`]); the client
///   keeps its locks for the whole command and refuses an output base a live foreign server owns.
/// - `daemon`: Bazel's client/server protocol. The server is a separate process started by the
///   install base's `daemonize`: this binary itself (`/proc/self/exe`), with the argument vector
///   `GetServerExeArgs` builds.
/// - `jvm` (tests only): as `daemon`, but the server executable is the one
///   `StartupOptions::GetExe` computes (the JVM of the install base), so a Bazel install base
///   gives a real Bazel server.
///
/// `--batch` runs in-process in every mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServerMode {
    InProcess,
    Daemon,
    Jvm,
}

impl ServerMode {
    /// The name of the environment variable.
    pub const ENV: &'static str = "BLAZE_SERVER_MODE";

    /// The mode for the value of `BLAZE_SERVER_MODE` (`None`: unset).
    pub fn parse(value: Option<&str>) -> Result<ServerMode, String> {
        match value {
            None | Some("inprocess") => Ok(ServerMode::InProcess),
            Some("daemon") => Ok(ServerMode::Daemon),
            Some("jvm") => Ok(ServerMode::Jvm),
            Some(other) => Err(format!(
                "Invalid value of {}: '{}'. Use 'inprocess' (or leave it unset), 'daemon' or \
                 'jvm'.",
                Self::ENV,
                other
            )),
        }
    }

    /// Reads `BLAZE_SERVER_MODE` from this process's environment.
    pub fn from_env() -> Result<ServerMode, String> {
        if crate::platform::exists_env(Self::ENV) {
            Self::parse(Some(&crate::platform::get_env(Self::ENV)))
        } else {
            Self::parse(None)
        }
    }

    /// Whether the server is a separate process the client talks to over gRPC.
    pub fn is_server_process(self) -> bool {
        self != ServerMode::InProcess
    }
}
