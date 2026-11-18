//! Generates the gRPC client and server stubs of the Remote Execution API services and of
//! `google.bytestream.ByteStream` with `tonic-build`'s manual builder: the message types are
//! `blaze_protos`' (compiled there with the Java-identical encoders), so only the service
//! descriptions live here. Java: the `*Grpc.java` classes generated from
//! `build/bazel/remote/execution/v2/remote_execution.proto` and `google/bytestream/bytestream.proto`.

use tonic_build::manual::{Builder, Method, Service};

const REAPI: &str = "blaze_protos::build::bazel::remote::execution::v2";
const BYTESTREAM: &str = "blaze_protos::google::bytestream";
const LONGRUNNING: &str = "blaze_protos::google::longrunning";

#[derive(Clone, Copy, PartialEq)]
enum Streaming {
    Unary,
    Server,
    Client,
}

fn method(name: &str, route: &str, input: String, output: String, streaming: Streaming) -> Method {
    let b = Method::builder()
        .name(name)
        .route_name(route)
        .input_type(input)
        .output_type(output)
        .codec_path("tonic_prost::ProstCodec");
    match streaming {
        Streaming::Unary => b.build(),
        Streaming::Server => b.server_streaming().build(),
        Streaming::Client => b.client_streaming().build(),
    }
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let re = |t: &str| format!("{REAPI}::{t}");
    let bs = |t: &str| format!("{BYTESTREAM}::{t}");
    let op = format!("{LONGRUNNING}::Operation");

    let execution = Service::builder()
        .name("Execution")
        .package("build.bazel.remote.execution.v2")
        .method(method(
            "execute",
            "Execute",
            re("ExecuteRequest"),
            op.clone(),
            Streaming::Server,
        ))
        .method(method(
            "wait_execution",
            "WaitExecution",
            re("WaitExecutionRequest"),
            op,
            Streaming::Server,
        ))
        .build();

    let action_cache = Service::builder()
        .name("ActionCache")
        .package("build.bazel.remote.execution.v2")
        .method(method(
            "get_action_result",
            "GetActionResult",
            re("GetActionResultRequest"),
            re("ActionResult"),
            Streaming::Unary,
        ))
        .method(method(
            "update_action_result",
            "UpdateActionResult",
            re("UpdateActionResultRequest"),
            re("ActionResult"),
            Streaming::Unary,
        ))
        .build();

    let cas = Service::builder()
        .name("ContentAddressableStorage")
        .package("build.bazel.remote.execution.v2")
        .method(method(
            "find_missing_blobs",
            "FindMissingBlobs",
            re("FindMissingBlobsRequest"),
            re("FindMissingBlobsResponse"),
            Streaming::Unary,
        ))
        .method(method(
            "batch_update_blobs",
            "BatchUpdateBlobs",
            re("BatchUpdateBlobsRequest"),
            re("BatchUpdateBlobsResponse"),
            Streaming::Unary,
        ))
        .method(method(
            "batch_read_blobs",
            "BatchReadBlobs",
            re("BatchReadBlobsRequest"),
            re("BatchReadBlobsResponse"),
            Streaming::Unary,
        ))
        .method(method(
            "get_tree",
            "GetTree",
            re("GetTreeRequest"),
            re("GetTreeResponse"),
            Streaming::Server,
        ))
        .method(method(
            "split_blob",
            "SplitBlob",
            re("SplitBlobRequest"),
            re("SplitBlobResponse"),
            Streaming::Unary,
        ))
        .method(method(
            "splice_blob",
            "SpliceBlob",
            re("SpliceBlobRequest"),
            re("SpliceBlobResponse"),
            Streaming::Unary,
        ))
        .build();

    let capabilities = Service::builder()
        .name("Capabilities")
        .package("build.bazel.remote.execution.v2")
        .method(method(
            "get_capabilities",
            "GetCapabilities",
            re("GetCapabilitiesRequest"),
            re("ServerCapabilities"),
            Streaming::Unary,
        ))
        .build();

    let byte_stream = Service::builder()
        .name("ByteStream")
        .package("google.bytestream")
        .method(method(
            "read",
            "Read",
            bs("ReadRequest"),
            bs("ReadResponse"),
            Streaming::Server,
        ))
        .method(method(
            "write",
            "Write",
            bs("WriteRequest"),
            bs("WriteResponse"),
            Streaming::Client,
        ))
        .method(method(
            "query_write_status",
            "QueryWriteStatus",
            bs("QueryWriteStatusRequest"),
            bs("QueryWriteStatusResponse"),
            Streaming::Unary,
        ))
        .build();

    Builder::new().compile(&[execution, action_cache, cas, capabilities, byte_stream]);
}
