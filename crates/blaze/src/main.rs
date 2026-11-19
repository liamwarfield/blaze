//! The `blaze` binary: the client (`src/main/cpp/main.cc`) with the server (`Bazel.main`) linked
//! in-process (docs/crates.md: no gRPC server until wave W8).

mod embedded;

use blaze_client::server::{InProcessServer, ServerStartup};
use blaze_protos::command_server::RunRequest;
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

    fn run_command(&self, startup: &ServerStartup, request: RunRequest) -> i32 {
        Self::init();
        BlazeRuntime::server_main_single_command(
            blaze_runtime::bazel_modules::bazel_modules(),
            startup.startup_args().to_vec(),
            request,
        )
    }

    // Java: com/google/devtools/build/lib/server/CommandManager.java:99 CommandManager.doCancel
    fn canceller(&self) -> std::sync::Arc<dyn Fn() + Send + Sync> {
        std::sync::Arc::new(blaze_runtime::interrupt::interrupt_command_thread)
    }
}

fn main() {
    // The raw bytes of each argument, as internal strings (one char per byte), as the C++
    // `main` gets them.
    // Java: src/main/cpp/main.cc:24 main
    let argv = blaze_client::argv_from_os();
    let code = blaze_client::main(argv, &BazelServer, &embedded::archive());
    std::process::exit(code);
}
