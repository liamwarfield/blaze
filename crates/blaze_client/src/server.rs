//! The client/server seam.
//!
//! Bazel's client either `exec`s the server JVM in `--batch` mode (`RunBatchMode`) or starts a
//! long-lived server and sends it one `command_server.RunRequest` per command over gRPC
//! (`RunClientServerMode` / `BlazeServer::Communicate`). Until wave W8 Blaze runs the server
//! in-process (docs/crates.md): the client builds exactly the arguments and the request the server
//! would receive and hands them to an [`InProcessServer`] instead of a JVM.

use blaze_protos::command_server::RunRequest;

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
    /// command, logging args, command arguments; cookie empty). Returns the exit code of the final
    /// `RunResponse`. The server shuts down when the command finishes.
    fn run_command(&self, startup: &ServerStartup, request: RunRequest) -> i32;
}
