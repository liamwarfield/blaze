//! Generates the gRPC server and client stubs of `command_server.CommandServer` with
//! `tonic-build`'s manual builder: the message types are `blaze_protos`' (compiled there with the
//! Java-identical encoders), so only the service description lives here. The codec is
//! `crate::grpc::CommandServerCodec` (prost, one HTTP/2 chunk per message).
//! Java: `CommandServerGrpc.java`, generated from `src/main/protobuf/command_server.proto:266`.

use tonic_build::manual::{Builder, Method, Service};

const PKG: &str = "blaze_protos::command_server";

fn method(name: &str, route: &str, input: &str, output: &str, server_streaming: bool) -> Method {
    let b = Method::builder()
        .name(name)
        .route_name(route)
        .input_type(format!("{PKG}::{input}"))
        .output_type(format!("{PKG}::{output}"))
        .codec_path("crate::grpc::CommandServerCodec");
    if server_streaming {
        b.server_streaming().build()
    } else {
        b.build()
    }
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let service = Service::builder()
        .name("CommandServer")
        .package("command_server")
        .method(method("run", "Run", "RunRequest", "RunResponse", true))
        .method(method(
            "cancel",
            "Cancel",
            "CancelRequest",
            "CancelResponse",
            false,
        ))
        .method(method(
            "update_terminal_size",
            "UpdateTerminalSize",
            "TerminalSizeRequest",
            "TerminalSizeResponse",
            false,
        ))
        .method(method("ping", "Ping", "PingRequest", "PingResponse", false))
        .build();
    Builder::new().compile(&[service]);
}
