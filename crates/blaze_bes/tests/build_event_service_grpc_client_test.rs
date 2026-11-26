//! Port of `BuildEventServiceGrpcClientTest`.
//!
//! PORT: there is no in-process gRPC transport; the fake server listens on `127.0.0.1:0` and the
//! client talks to it over a plaintext channel (`blaze_remote::grpc::channel::testing`).
// Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceGrpcClientTest.java

use std::pin::Pin;
use std::sync::Arc;

use blaze_bes::client::grpc_client::BuildEventServiceGrpcClient;
use blaze_bes::client::{BuildEventServiceClient, CommandContext};
use blaze_bes::stubs::publish_build_event_server::{PublishBuildEvent, PublishBuildEventServer};
use blaze_protos::google::devtools::build::v1::{
    PublishBuildToolEventStreamRequest, PublishBuildToolEventStreamResponse,
    PublishLifecycleEventRequest,
};
use blaze_protos::google::protobuf::Empty;
use blaze_remote::grpc::channel::testing::channel_to;
use blaze_remote::grpc::channel::{AttachHeadersInterceptor, ClientInterceptor};
use indexmap::IndexSet;
use parking_lot::Mutex;
use tokio_stream::Stream;
use tonic::metadata::MetadataMap;
use tonic::{Request, Response, Status, Streaming};

fn command_context() -> CommandContext {
    CommandContext::new(
        "8d720129-bc1a-481f-8c4c-dbe11dcef319".to_string(),
        "1b4c3b8e-6c3f-4f0a-9f0e-0c6b0d7a5e21".to_string(),
        1,
        IndexSet::new(),
        None,
        false,
    )
}

/// What the fake service does with a stream.
#[derive(Clone, Copy)]
enum Behavior {
    /// `NOOP_SERVER`: `responseObserver.onCompleted()`.
    Complete,
    /// `responseObserver.onError(new StatusException(Status.INTERNAL))`.
    FailInternal,
}

#[derive(Clone)]
struct FakeService {
    behavior: Behavior,
    seen_headers: Arc<Mutex<Vec<MetadataMap>>>,
}

type ResponseStream =
    Pin<Box<dyn Stream<Item = Result<PublishBuildToolEventStreamResponse, Status>> + Send>>;

#[tonic::async_trait]
impl PublishBuildEvent for FakeService {
    async fn publish_lifecycle_event(
        &self,
        _request: Request<PublishLifecycleEventRequest>,
    ) -> Result<Response<Empty>, Status> {
        Err(Status::unimplemented(""))
    }

    type PublishBuildToolEventStreamStream = ResponseStream;

    async fn publish_build_tool_event_stream(
        &self,
        request: Request<Streaming<PublishBuildToolEventStreamRequest>>,
    ) -> Result<Response<ResponseStream>, Status> {
        self.seen_headers.lock().push(request.metadata().clone());
        match self.behavior {
            Behavior::Complete => {
                let empty: ResponseStream = Box::pin(tokio_stream::empty());
                Ok(Response::new(empty))
            }
            Behavior::FailInternal => Err(Status::internal("")),
        }
    }
}

/// Starts the fake service on `127.0.0.1:0`; returns its address. The server runs until the
/// test process ends.
fn start_test_server(service: FakeService) -> std::net::SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("address");
    listener.set_nonblocking(true).expect("non-blocking");
    blaze_remote::runtime::runtime().spawn(async move {
        let listener = tokio::net::TcpListener::from_std(listener).expect("listener");
        tonic::transport::Server::builder()
            .serve_with_incoming(
                PublishBuildEventServer::new(service),
                tonic::transport::server::TcpIncoming::from(listener),
            )
            .await
            .expect("fake server failed");
    });
    addr
}

fn client(
    addr: std::net::SocketAddr,
    interceptors: Vec<Arc<dyn ClientInterceptor>>,
) -> BuildEventServiceGrpcClient {
    BuildEventServiceGrpcClient::new(channel_to(addr, interceptors, None), None)
}

#[test]
fn bes_headers() {
    let seen_headers = Arc::new(Mutex::new(Vec::new()));
    let addr = start_test_server(FakeService {
        behavior: Behavior::Complete,
        seen_headers: seen_headers.clone(),
    });
    let interceptor: Arc<dyn ClientInterceptor> = Arc::new(
        AttachHeadersInterceptor::new(&[("metadata-foo".to_string(), "bar".to_string())])
            .expect("valid header"),
    );
    let grpc_client = client(addr, vec![interceptor]);
    let status = grpc_client
        .open_stream(&command_context(), Arc::new(|_ack| {}))
        .expect("not interrupted")
        .get_status()
        .get();
    assert_eq!(status.code(), tonic::Code::Ok);
    let seen_headers = seen_headers.lock();
    assert_eq!(seen_headers.len(), 1);
    assert_eq!(
        seen_headers[0]
            .get("metadata-foo")
            .map(|v| v.to_str().unwrap()),
        Some("bar")
    );
}

#[test]
fn immediate_success() {
    let addr = start_test_server(FakeService {
        behavior: Behavior::Complete,
        seen_headers: Arc::default(),
    });
    let status = client(addr, vec![])
        .open_stream(&command_context(), Arc::new(|_ack| {}))
        .expect("not interrupted")
        .get_status()
        .get();
    assert_eq!(status.code(), tonic::Code::Ok);
}

#[test]
fn immediate_failure() {
    let addr = start_test_server(FakeService {
        behavior: Behavior::FailInternal,
        seen_headers: Arc::default(),
    });
    let status = client(addr, vec![])
        .open_stream(&command_context(), Arc::new(|_ack| {}))
        .expect("not interrupted")
        .get_status()
        .get();
    // Status.INTERNAL: the code, without a description.
    assert_eq!(status.code(), tonic::Code::Internal);
    assert_eq!(status.message(), "");
}
