//! The `blaze` binary: the client (`src/main/cpp/main.cc`) and the server (`Bazel.main`) in one
//! executable.
//!
//! The client starts this binary as its server (`BLAZE_SERVER_MODE=daemon`, or Bazel's own
//! client with `--server_javabase=<dir>` whose `bin/java` is this binary) with the argument
//! vector `GetServerExeArgs` builds for the JVM; `main` recognises that vector and runs
//! `Bazel.main`. Otherwise it is the client, whose default server runs in this process
//! ([`InProcessServer`]).

mod embedded;

use std::sync::Arc;

use blaze_client::server::{InProcessServer, ServerStartup};
use blaze_protos::command_server::{RunRequest, RunResponse};
use blaze_runtime::runtime::BlazeRuntime;
use blaze_runtime::version_info::{BlazeVersionInfo, try_get_build_info};

/// `Bazel.main`: the Bazel modules and `BlazeRuntime.main`, called in-process by the client.
// Java: com/google/devtools/build/lib/bazel/Bazel.java:0 Bazel.main
struct BazelServer;

impl BazelServer {
    // Java: com/google/devtools/build/lib/bazel/Bazel.java:109 Bazel.main
    fn init() {
        BlazeVersionInfo::set_build_info(try_get_build_info());
    }
}

impl InProcessServer for BazelServer {
    fn batch_main(&self, _startup: &ServerStartup, args: Vec<String>) -> i32 {
        Self::init();
        BlazeRuntime::batch_main(blaze_runtime::bazel_modules::bazel_modules(), args)
    }

    fn run_command(&self, startup: &ServerStartup, request: RunRequest) -> RunResponse {
        Self::init();
        BlazeRuntime::server_main_single_command(
            blaze_runtime::bazel_modules::bazel_modules(),
            startup.startup_args().to_vec(),
            request,
        )
    }

    // Java: com/google/devtools/build/lib/server/CommandManager.java:99 CommandManager.doCancel
    fn canceller(&self) -> Arc<dyn Fn() + Send + Sync> {
        // The client calls this on the thread that then runs the command (`run_command`).
        blaze_runtime::interrupt::command_thread_canceller()
    }
}

/// The `String[] args` of the server's `main` if `argv` is the argument vector a client starts
/// its server with (`GetServerExeArgs`: `argv[0]`, the JVM flags, `-jar <server jar>`, then the
/// startup options and, in `--batch` mode, the command): the arguments after `-jar <path>`.
///
/// PORT: Blaze only (lead's rule). A JVM reads its own flags up to `-jar`; the vector is a
/// server start iff some argument equals `-jar` and every argument between `argv[0]` and it
/// starts with `-` (every JVM flag `GetServerExeArgs` writes does, and no client invocation of
/// `blaze` starts with such a sequence followed by `-jar`).
fn server_args(argv: &[String]) -> Option<Vec<String>> {
    let jar = argv.iter().skip(1).position(|a| a == "-jar")? + 1;
    if !argv[1..jar].iter().all(|a| a.starts_with('-')) {
        return None;
    }
    // `-jar` is followed by the jar's path; the JVM's `main` gets what comes after it.
    if jar + 1 >= argv.len() {
        return None;
    }
    Some(argv[jar + 2..].to_vec())
}

fn main() {
    // The raw bytes of each argument, as internal strings (one char per byte), as the C++
    // `main` gets them.
    // Java: src/main/cpp/main.cc:24 main
    let argv = blaze_client::argv_from_os();
    if let Some(args) = server_args(&argv) {
        // The JVM: `Bazel.main(args)`.
        // Java: com/google/devtools/build/lib/bazel/Bazel.java:109 Bazel.main
        BazelServer::init();
        BlazeRuntime::main(blaze_runtime::bazel_modules::bazel_modules(), args);
    }
    // The in-process server's stdout and stderr are the client's: an EPIPE there is what the
    // client's SIGPIPE handler records (blaze_events::io::set_broken_pipe_listener).
    blaze_events::io::set_broken_pipe_listener(blaze_client::signal_handler::note_broken_pipe);
    let code = blaze_client::main(argv, &BazelServer, &embedded::archive());
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::server_args;

    fn v(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| a.to_string()).collect()
    }

    #[test]
    fn server_args_after_the_jar() {
        assert_eq!(
            server_args(&v(&[
                "bazel(ws)",
                "--add-opens=java.base/java.lang=ALL-UNNAMED",
                "-Xverify:none",
                "-Djava.library.path=/ib",
                "-jar",
                "/ib/A-server.jar",
                "--max_idle_secs=10800",
                "--output_base=/ob",
            ])),
            Some(v(&["--max_idle_secs=10800", "--output_base=/ob"]))
        );
        assert_eq!(
            server_args(&v(&[
                "java", "-jar", "/j", "--batch", "--x", "build", "//:a"
            ])),
            Some(v(&["--batch", "--x", "build", "//:a"]))
        );
    }

    #[test]
    fn client_invocations_are_not_server_starts() {
        assert_eq!(server_args(&v(&["blaze", "build", "-jar", "x"])), None);
        assert_eq!(
            server_args(&v(&["blaze", "--batch", "build", "//:a"])),
            None
        );
        assert_eq!(server_args(&v(&["blaze"])), None);
        assert_eq!(server_args(&v(&["blaze", "-jar"])), None);
    }
}
