//! Generates the gRPC stubs of `google.devtools.build.v1.PublishBuildEvent` with `tonic-build`'s
//! manual builder: the message types are `blaze_protos`' (compiled there with the Java-identical
//! encoders), so only the service description lives here. The client side of Blaze goes through
//! `blaze_remote`'s channel; the generated server is used by the in-process fake servers of the
//! tests and by `blaze-bes-logging-server`. Java: the generated `PublishBuildEventGrpc` class.

use tonic_build::manual::{Builder, Method, Service};

const V1: &str = "blaze_protos::google::devtools::build::v1";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let service = Service::builder()
        .name("PublishBuildEvent")
        .package("google.devtools.build.v1")
        .method(
            Method::builder()
                .name("publish_lifecycle_event")
                .route_name("PublishLifecycleEvent")
                .input_type(format!("{V1}::PublishLifecycleEventRequest"))
                .output_type("blaze_protos::google::protobuf::Empty")
                .codec_path("tonic_prost::ProstCodec")
                .build(),
        )
        .method(
            Method::builder()
                .name("publish_build_tool_event_stream")
                .route_name("PublishBuildToolEventStream")
                .input_type(format!("{V1}::PublishBuildToolEventStreamRequest"))
                .output_type(format!("{V1}::PublishBuildToolEventStreamResponse"))
                .codec_path("tonic_prost::ProstCodec")
                .client_streaming()
                .server_streaming()
                .build(),
        )
        .build();
    Builder::new().compile(&[service]);
}
