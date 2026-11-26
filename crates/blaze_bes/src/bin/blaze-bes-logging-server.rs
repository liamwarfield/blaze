//! A Build Event Service that records every request it receives: the tool behind the oracle
//! comparison of the Build Event Service client (`--bes_backend`).
//!
//! Usage: `blaze-bes-logging-server <port> <log file> [fail-first-stream]`
//!
//! Listens on `127.0.0.1:<port>` (0 picks a free port, printed as `port=<n>` on stdout), answers
//! `PublishLifecycleEvent` with `Empty` and acknowledges every event of a
//! `PublishBuildToolEventStream`, and appends each request to the log file in text format,
//! preceded by `=== <method>` and the request headers that do not vary between runs. The
//! `bazel_event` payload is decoded and its event id printed. With `fail-first-stream` the first
//! stream is failed with `UNAVAILABLE` after its first event (to see the retries). Runs until
//! killed.

use std::io::Write;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use blaze_bes::stubs::publish_build_event_server::{PublishBuildEvent, PublishBuildEventServer};
use blaze_protos::build_event_stream::BuildEvent as BepEvent;
use blaze_protos::google::devtools::build::v1::build_event::Event;
use blaze_protos::google::devtools::build::v1::{
    PublishBuildToolEventStreamRequest, PublishBuildToolEventStreamResponse,
    PublishLifecycleEventRequest,
};
use blaze_protos::google::protobuf::Empty;
use parking_lot::Mutex;
use prost::Message as _;
use tokio_stream::Stream;
use tokio_stream::wrappers::ReceiverStream;
use tonic::metadata::MetadataMap;
use tonic::{Request, Response, Status, Streaming};

const USAGE: &str = "usage: blaze-bes-logging-server <port> <log file> [fail-first-stream]\n\
    Records every Build Event Service request in <log file> (text format) and acknowledges the\n\
    events. Port 0 picks a free port (printed as port=<n>). Runs until killed.";

#[derive(Clone)]
struct Logger {
    out: Arc<Mutex<std::fs::File>>,
    fail_first_stream: Arc<AtomicBool>,
}

/// The headers worth comparing: the request metadata and the `--bes_header`s (not the HTTP/2
/// and user-agent ones).
fn headers_text(metadata: &MetadataMap) -> String {
    let mut out = String::new();
    for kv in metadata.iter() {
        match kv {
            tonic::metadata::KeyAndValueRef::Ascii(k, v) => {
                let k = k.as_str();
                if matches!(
                    k,
                    "user-agent" | "te" | "content-type" | "grpc-accept-encoding" | "grpc-timeout"
                ) {
                    continue;
                }
                out.push_str(&format!("header {k}: {}\n", v.to_str().unwrap_or("?")));
            }
            tonic::metadata::KeyAndValueRef::Binary(k, v) => {
                let k = k.as_str();
                if k == "build.bazel.remote.execution.v2.requestmetadata-bin" {
                    let bytes = v.to_bytes().map(|b| b.to_vec()).unwrap_or_default();
                    match blaze_protos::build::bazel::remote::execution::v2::RequestMetadata::decode(
                        bytes.as_slice(),
                    ) {
                        Ok(m) => out.push_str(&format!(
                            "header {k}: {}\n",
                            blaze_protos::text_format::message_to_string(&m).replace('\n', " ")
                        )),
                        Err(_) => out.push_str(&format!("header {k}: <undecodable>\n")),
                    }
                } else {
                    out.push_str(&format!("header {k}: <binary>\n"));
                }
            }
        }
    }
    out
}

impl Logger {
    fn log(&self, text: &str) {
        let mut out = self.out.lock();
        let _ = out.write_all(text.as_bytes());
        let _ = out.flush();
    }
}

fn stream_request_text(request: &PublishBuildToolEventStreamRequest) -> String {
    let mut text = blaze_protos::text_format::message_to_string(request);
    if let Some(Event::BazelEvent(any)) = request
        .ordered_build_event
        .as_ref()
        .and_then(|o| o.event.as_ref())
        .and_then(|e| e.event.as_ref())
    {
        match BepEvent::decode(any.value.as_slice()) {
            Ok(event) => text.push_str(&format!(
                "bazel_event_id: {}\n",
                blaze_protos::text_format::message_to_string(&event.id.unwrap_or_default())
                    .replace('\n', " ")
            )),
            Err(e) => text.push_str(&format!("bazel_event_id: <undecodable: {e}>\n")),
        }
    }
    text
}

#[tonic::async_trait]
impl PublishBuildEvent for Logger {
    async fn publish_lifecycle_event(
        &self,
        request: Request<PublishLifecycleEventRequest>,
    ) -> Result<Response<Empty>, Status> {
        let headers = headers_text(request.metadata());
        let message = request.into_inner();
        self.log(&format!(
            "=== PublishLifecycleEvent\n{headers}{}",
            blaze_protos::text_format::message_to_string(&message)
        ));
        Ok(Response::new(Empty {}))
    }

    type PublishBuildToolEventStreamStream =
        Pin<Box<dyn Stream<Item = Result<PublishBuildToolEventStreamResponse, Status>> + Send>>;

    async fn publish_build_tool_event_stream(
        &self,
        request: Request<Streaming<PublishBuildToolEventStreamRequest>>,
    ) -> Result<Response<Self::PublishBuildToolEventStreamStream>, Status> {
        self.log(&format!(
            "=== PublishBuildToolEventStream (open)\n{}",
            headers_text(request.metadata())
        ));
        let fail_this_stream = self.fail_first_stream.swap(false, Ordering::SeqCst);
        let mut requests = request.into_inner();
        let (sender, receiver) = tokio::sync::mpsc::channel(64);
        let logger = self.clone();
        tokio::spawn(async move {
            loop {
                match requests.message().await {
                    Ok(Some(request)) => {
                        logger.log(&format!(
                            "=== PublishBuildToolEventStream (request)\n{}",
                            stream_request_text(&request)
                        ));
                        if fail_this_stream {
                            logger.log("=== PublishBuildToolEventStream (failing: UNAVAILABLE)\n");
                            let _ = sender
                                .send(Err(Status::unavailable("fail-first-stream")))
                                .await;
                            return;
                        }
                        let ordered = request.ordered_build_event.unwrap_or_default();
                        let response = PublishBuildToolEventStreamResponse {
                            stream_id: ordered.stream_id,
                            sequence_number: ordered.sequence_number,
                        };
                        if sender.send(Ok(response)).await.is_err() {
                            return;
                        }
                    }
                    Ok(None) => {
                        logger.log("=== PublishBuildToolEventStream (half-closed)\n");
                        return;
                    }
                    Err(status) => {
                        logger.log(&format!(
                            "=== PublishBuildToolEventStream (error: {:?})\n",
                            status.code()
                        ));
                        return;
                    }
                }
            }
        });
        Ok(Response::new(Box::pin(ReceiverStream::new(receiver))))
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args[0] == "help" || args[0] == "--help" || args.len() < 2 {
        println!("{USAGE}");
        return;
    }
    let port: u16 = args[0].parse().unwrap_or_else(|_| {
        eprintln!("{USAGE}");
        std::process::exit(2)
    });
    let out = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&args[1])
        .unwrap_or_else(|e| {
            eprintln!("cannot open {}: {e}", args[1]);
            std::process::exit(2)
        });
    let logger = Logger {
        out: Arc::new(Mutex::new(out)),
        fail_first_stream: Arc::new(AtomicBool::new(
            args.get(2).is_some_and(|a| a == "fail-first-stream"),
        )),
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("tokio runtime");
    runtime.block_on(async move {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .unwrap_or_else(|e| {
                eprintln!("cannot listen on port {port}: {e}");
                std::process::exit(2)
            });
        let addr = listener.local_addr().expect("local address");
        println!("port={}", addr.port());
        let _ = std::io::stdout().flush();
        // A single service: no router needed.
        tonic::transport::Server::builder()
            .serve_with_incoming(
                PublishBuildEventServer::new(logger),
                tonic::transport::server::TcpIncoming::from(listener),
            )
            .await
            .expect("server");
    });
}
