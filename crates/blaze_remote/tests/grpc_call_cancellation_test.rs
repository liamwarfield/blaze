//! Not a port of a Java test: a call the client gives up must reach the server as a cancel, as
//! grpc-java's `ClientCall.cancel` (a dropped future, an interrupt, BES's `abortStream`) resets
//! the HTTP/2 stream (`RST_STREAM CANCEL`) and the server's call fails with `CANCELLED`. The calls
//! here are the channel's streaming calls with the request side still open (ByteStream writes,
//! the Build Event Service stream) and a server-streaming call (Read; Execute and WaitExecution
//! are the same call kind). A normal half-close still reaches the server as the end of the
//! requests.
//!
//! How the fake server sees a reset: tonic's server `Streaming` reports a `CANCELLED` request
//! stream as its end (tonic `codec/decode.rs`), so the request stream cannot tell a reset from a
//! half-close; but on a reset hyper drops the call's handler future and its response body,
//! which it does not on a half-close. The servers here record that drop.

mod util;

use std::convert::Infallible;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use blaze_remote::grpc::channel::{CallOptions, methods};
use blaze_remote::reapi::bytestream::{ReadRequest, ReadResponse, WriteRequest, WriteResponse};
use blaze_remote::runtime::block_on;
use futures::Stream;
use futures::future::BoxFuture;
use parking_lot::Mutex;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tonic::body::Body;
use tonic::{Status, Streaming};
use tonic_prost::ProstCodec;

use util::grpc_server::{FakeByteStream, FakeServer, hanging_response_stream};

/// What the server's call saw, in order: `message` per request, `end` for the end of the
/// requests, `dropped` when hyper dropped the call (a reset).
#[derive(Default)]
struct ServerLog(Mutex<Vec<&'static str>>);

impl ServerLog {
    fn push(&self, event: &'static str) {
        self.0.lock().push(event);
    }

    fn events(&self) -> Vec<&'static str> {
        self.0.lock().clone()
    }

    /// Waits up to `limit` for `pred` (the cancel travels over a socket).
    fn wait_for(
        &self,
        limit: Duration,
        pred: impl Fn(&[&'static str]) -> bool,
    ) -> Vec<&'static str> {
        let start = Instant::now();
        loop {
            let events = self.events();
            if pred(&events) || start.elapsed() > limit {
                return events;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

/// Pushes `dropped` when dropped while armed.
struct DropLog(Arc<ServerLog>, bool);

impl Drop for DropLog {
    fn drop(&mut self) {
        if self.1 {
            self.0.push("dropped");
        }
    }
}

/// A server whose `ByteStream.Write` reads the requests to their end; then it answers
/// (`respond`) or waits forever, so that only a reset ends the call.
fn write_server(log: Arc<ServerLog>, respond: bool) -> FakeServer {
    let mut server = FakeServer::new();
    server.service_registry.add_byte_stream(FakeByteStream {
        write: Some(Box::new(move |request| {
            let log = log.clone();
            Box::pin(async move {
                let mut guard = DropLog(log.clone(), true);
                let mut requests = request.into_inner();
                loop {
                    match requests.message().await {
                        Ok(Some(_)) => log.push("message"),
                        Ok(None) => {
                            log.push("end");
                            if respond {
                                guard.1 = false;
                                return Ok(WriteResponse { committed_size: 1 });
                            }
                            futures::future::pending::<()>().await;
                        }
                        Err(status) => {
                            log.push("error");
                            guard.1 = false;
                            return Err(status);
                        }
                    }
                }
            })
        })),
        ..Default::default()
    });
    server.start();
    server
}

/// A bidi method on the `ByteStream` service name: it answers the headers at once (as a BES
/// server does), logs the requests in a task of its own, and sends no response until the call
/// ends; `dropped` is pushed when hyper drops the response stream.
#[derive(Clone)]
struct BidiWrite(Arc<ServerLog>);

impl tower::Service<http::Request<Body>> for BidiWrite {
    type Response = http::Response<Body>;
    type Error = Infallible;
    type Future = BoxFuture<'static, Result<http::Response<Body>, Infallible>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Infallible>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: http::Request<Body>) -> Self::Future {
        let log = self.0.clone();
        Box::pin(async move {
            let codec: ProstCodec<WriteResponse, WriteRequest> = ProstCodec::default();
            Ok(tonic::server::Grpc::new(codec)
                .streaming(BidiHandler(log), request)
                .await)
        })
    }
}

struct BidiHandler(Arc<ServerLog>);

type BoxStream = Pin<Box<dyn Stream<Item = Result<WriteResponse, Status>> + Send>>;

impl tonic::server::StreamingService<WriteRequest> for BidiHandler {
    type Response = WriteResponse;
    type ResponseStream = BoxStream;
    type Future = BoxFuture<'static, Result<tonic::Response<BoxStream>, Status>>;

    fn call(&mut self, request: tonic::Request<Streaming<WriteRequest>>) -> Self::Future {
        let log = self.0.clone();
        Box::pin(async move {
            let mut requests = request.into_inner();
            let reader_log = log.clone();
            tokio::spawn(async move {
                loop {
                    match requests.message().await {
                        Ok(Some(_)) => reader_log.push("message"),
                        Ok(None) => return reader_log.push("end"),
                        Err(_) => return reader_log.push("error"),
                    }
                }
            });
            let responses: BoxStream = Box::pin(DroppedStream(DropLog(log, true)));
            Ok(tonic::Response::new(responses))
        })
    }
}

/// A response stream that never yields and logs its drop.
struct DroppedStream(DropLog);

impl Stream for DroppedStream {
    type Item = Result<WriteResponse, Status>;

    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Pending
    }
}

fn bidi_server(log: Arc<ServerLog>) -> FakeServer {
    let mut server = FakeServer::new();
    server
        .service_registry
        .add("google.bytestream.ByteStream", BidiWrite(log));
    server.start();
    server
}

fn write_request() -> WriteRequest {
    WriteRequest {
        resource_name: "uploads/1/blobs/abc/1".to_string(),
        data: b"a".to_vec(),
        ..Default::default()
    }
}

/// BES's `abortStream`: the response stream of a bidi call (headers received) is dropped while
/// the request side is open.
#[test]
fn dropping_a_bidi_response_stream_with_open_requests_resets_the_call() {
    let log = Arc::new(ServerLog::default());
    let server = bidi_server(log.clone());
    let channel = server.channel(vec![], None);
    let (sender, receiver) = mpsc::unbounded_channel();
    sender.send(write_request()).unwrap();

    let responses = block_on(channel.bidi_streaming::<_, WriteResponse, _>(
        methods::WRITE,
        UnboundedReceiverStream::new(receiver),
        CallOptions::new(),
    ))
    .unwrap();
    assert_eq!(
        log.wait_for(Duration::from_secs(5), |e| !e.is_empty()),
        vec!["message"]
    );
    drop(responses);

    let events = log.wait_for(Duration::from_secs(5), |e| e.contains(&"dropped"));
    assert!(events.contains(&"dropped"), "{events:?}");
    // The request side was still open: the reset did not wait for it to end.
    drop(sender);
}

/// An interrupt while a bidi call waits for its headers: its future is dropped while the
/// request side is open.
#[test]
fn dropping_a_bidi_call_future_with_open_requests_resets_the_call() {
    let log = Arc::new(ServerLog::default());
    let server = write_server(log.clone(), false);
    let channel = server.channel(vec![], None);
    let (sender, receiver) = mpsc::unbounded_channel();
    sender.send(write_request()).unwrap();

    drive_until_message(
        &log,
        channel.bidi_streaming::<_, WriteResponse, _>(
            methods::WRITE,
            UnboundedReceiverStream::new(receiver),
            CallOptions::new(),
        ),
    );

    let events = log.wait_for(Duration::from_secs(5), |e| e.contains(&"dropped"));
    assert!(events.contains(&"dropped"), "{events:?}");
    drop(sender);
}

/// An interrupt of a ByteStream write: the future of a client-streaming call is dropped while
/// the request side is open.
#[test]
fn dropping_a_client_streaming_call_with_open_requests_resets_the_call() {
    let log = Arc::new(ServerLog::default());
    let server = write_server(log.clone(), false);
    let channel = server.channel(vec![], None);
    let (sender, receiver) = mpsc::unbounded_channel();
    sender.send(write_request()).unwrap();

    drive_until_message(
        &log,
        channel.client_streaming::<_, WriteResponse, _>(
            methods::WRITE,
            UnboundedReceiverStream::new(receiver),
            CallOptions::new(),
        ),
    );

    let events = log.wait_for(Duration::from_secs(5), |e| e.contains(&"dropped"));
    assert!(events.contains(&"dropped"), "{events:?}");
    drop(sender);
}

/// Polls `call` until the server logged a request, then drops it.
fn drive_until_message<T>(log: &ServerLog, call: impl std::future::Future<Output = T>) {
    block_on(async {
        tokio::pin!(call);
        let start = Instant::now();
        loop {
            tokio::select! {
                biased;
                _ = &mut call => panic!("the call finished"),
                _ = tokio::time::sleep(Duration::from_millis(5)) => {
                    if !log.events().is_empty() {
                        break;
                    }
                    assert!(start.elapsed() < Duration::from_secs(5), "no request reached the server");
                }
            }
        }
        // The call's future is dropped here.
    });
}

/// The end of the request stream is a half-close, not a reset: the server's call goes on.
#[test]
fn ending_the_requests_half_closes_the_call() {
    // A server that answers after the half-close.
    let log = Arc::new(ServerLog::default());
    let server = write_server(log.clone(), true);
    let channel = server.channel(vec![], None);
    let response = block_on(channel.client_streaming::<_, WriteResponse, _>(
        methods::WRITE,
        futures::stream::iter(vec![write_request()]),
        CallOptions::new(),
    ))
    .unwrap();
    assert_eq!(response.committed_size, 1);
    assert_eq!(log.events(), vec!["message", "end"]);

    // A bidi call whose requests end while the server's call stays open.
    let log = Arc::new(ServerLog::default());
    let server = bidi_server(log.clone());
    let channel = server.channel(vec![], None);
    let responses = block_on(channel.bidi_streaming::<_, WriteResponse, _>(
        methods::WRITE,
        futures::stream::iter(vec![write_request()]),
        CallOptions::new(),
    ))
    .unwrap();
    let events = log.wait_for(Duration::from_secs(5), |e| e.len() >= 2);
    assert_eq!(events, vec!["message", "end"]);
    // Not reset while the client still reads.
    let events = log.wait_for(Duration::from_millis(300), |e| e.contains(&"dropped"));
    assert_eq!(events, vec!["message", "end"]);
    drop(responses);
}

/// An interrupt of a server-streaming call: dropping the response stream ends the server's call.
#[test]
fn dropping_a_server_streaming_call_resets_the_call() {
    let cancelled = Arc::new(AtomicBool::new(false));
    let started = Arc::new(AtomicBool::new(false));
    let mut server = FakeServer::new();
    {
        let cancelled = cancelled.clone();
        let started = started.clone();
        server.service_registry.add_byte_stream(FakeByteStream {
            read: Some(Box::new(move |_request| {
                started.store(true, Ordering::SeqCst);
                hanging_response_stream(vec![], cancelled.clone())
            })),
            ..Default::default()
        });
    }
    server.start();
    let channel = server.channel(vec![], None);

    let responses = block_on(channel.server_streaming::<_, ReadResponse>(
        methods::READ,
        ReadRequest {
            resource_name: "blobs/abc/1".to_string(),
            ..Default::default()
        },
        CallOptions::new(),
    ))
    .unwrap();
    let start = Instant::now();
    while !started.load(Ordering::SeqCst) && start.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(started.load(Ordering::SeqCst));
    drop(responses);

    let start = Instant::now();
    while !cancelled.load(Ordering::SeqCst) && start.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(cancelled.load(Ordering::SeqCst));
}
