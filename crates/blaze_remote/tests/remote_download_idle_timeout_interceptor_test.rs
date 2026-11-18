#![allow(clippy::field_reassign_with_default)]
//! Port of
//! `src/test/java/com/google/devtools/build/lib/remote/RemoteDownloadIdleTimeoutInterceptorTest.java`.
//!
//! PORT: Java's `RemoteDownloadIdleTimeoutInterceptor` is a channel interceptor driven by a
//! `ScheduledExecutorService` and a `nanoTime` supplier, and the Java tests call it directly
//! with a fake `ClientCall` and a mock scheduler. In Rust the interceptor is
//! `ReferenceCountedChannel::with_download_idle_timeout`, applied by the channel's
//! `server_streaming` call and by `ResponseStream::message`; it has no scheduler or clock to
//! inject. The tests whose behaviour is observable through a real call run against a fake
//! ByteStream server on localhost (`util::grpc_server`) with real time; the tests of the
//! scheduler bookkeeping are ignored.
//!
//! Java's `FakeClientCall.cancelled` (the interceptor cancelled the call) is observed here as
//! the server seeing its call go away (`hanging_response_stream`'s `cancelled` flag); Java's
//! listener `onClose(status)` is observed through a channel-level `ClientInterceptor`.

mod util;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use blaze_remote::grpc::channel::java_duration_to_string;
use blaze_remote::grpc::{
    CallListener, CallOptions, ClientInterceptor, Method, ReferenceCountedChannel, methods,
};
use blaze_remote::reapi::bytestream::{ReadRequest, ReadResponse, WriteRequest, WriteResponse};
use blaze_remote::reapi::{Digest, GetTreeRequest, GetTreeResponse};
use futures::StreamExt;
use futures::future::BoxFuture;
use parking_lot::Mutex;
use tonic::metadata::MetadataMap;

use util::grpc_server::{
    FakeByteStream, FakeContentAddressableStorage, FakeServer, hanging_response_stream,
    read_response,
};

type WriteFuture = BoxFuture<'static, Result<WriteResponse, tonic::Status>>;

/// The statuses a call's listener was closed with (Java: the `onClose` override of the test
/// listener), per method.
#[derive(Clone, Default)]
struct ClosedStatuses {
    closed: Arc<Mutex<Vec<(Method, tonic::Code, String)>>>,
}

struct ClosedStatusListener {
    method: Method,
    closed: Arc<Mutex<Vec<(Method, tonic::Code, String)>>>,
}

impl CallListener for ClosedStatusListener {
    fn on_close(&mut self, status: &tonic::Status) {
        self.closed
            .lock()
            .push((self.method, status.code(), status.message().to_string()));
    }
}

impl ClientInterceptor for ClosedStatuses {
    fn intercept_call(
        &self,
        method: &Method,
        _headers: &mut MetadataMap,
    ) -> Option<Box<dyn CallListener>> {
        Some(Box::new(ClosedStatusListener {
            method: *method,
            closed: self.closed.clone(),
        }))
    }
}

impl ClosedStatuses {
    fn get(&self) -> Vec<(Method, tonic::Code, String)> {
        self.closed.lock().clone()
    }
}

/// A channel to `server` with the download idle timeout and the close recorder.
fn channel_with_idle_timeout(
    server: &FakeServer,
    closed: &ClosedStatuses,
    timeout: Duration,
) -> ReferenceCountedChannel {
    let interceptor: Arc<dyn ClientInterceptor> = Arc::new(closed.clone());
    server
        .channel(vec![interceptor], None)
        .with_download_idle_timeout(timeout)
}

/// Reads every message of a `ByteStream.Read` call.
async fn read_all(channel: &ReferenceCountedChannel) -> Result<Vec<ReadResponse>, tonic::Status> {
    let mut stream = channel
        .server_streaming::<ReadRequest, ReadResponse>(
            methods::READ,
            ReadRequest::default(),
            CallOptions::new(),
        )
        .await?;
    let mut responses = Vec::new();
    while let Some(response) = stream.message().await? {
        responses.push(response);
    }
    Ok(responses)
}

/// Waits (up to 10s) until the server saw its call go away.
async fn await_cancelled(cancelled: &AtomicBool) -> bool {
    for _ in 0..200 {
        if cancelled.load(Ordering::SeqCst) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    cancelled.load(Ordering::SeqCst)
}

/// A server stream that sends `items` with `delay` before each one, then completes.
fn delayed_stream<T: Send + 'static>(
    items: Vec<T>,
    delay: Duration,
) -> util::grpc_server::ResponseStream<T> {
    Box::pin(futures::stream::iter(items).then(move |item| async move {
        tokio::time::sleep(delay).await;
        Ok::<T, tonic::Status>(item)
    }))
}

#[test]
fn byte_stream_write_does_not_schedule_timeout() {
    // ByteStream.Write is an upload RPC, so it must not inherit the download idle timeout even
    // though it belongs to the same gRPC service as ByteStream.Read.
    //
    // PORT: Java verifies the scheduler is never asked; here the server answers the Write only
    // after twice the idle timeout, and the call must still succeed.
    let timeout = Duration::from_secs(1);
    let mut server = FakeServer::new();
    server.service_registry.add_byte_stream(FakeByteStream {
        write: Some(Box::new(
            move |request: tonic::Request<tonic::Streaming<WriteRequest>>| -> WriteFuture {
                Box::pin(async move {
                    let mut requests = request.into_inner();
                    while let Some(r) = requests.next().await {
                        r?;
                    }
                    tokio::time::sleep(timeout * 2).await;
                    Ok(WriteResponse { committed_size: 0 })
                })
            },
        )),
        ..Default::default()
    });
    server.start();
    let closed = ClosedStatuses::default();
    let channel = channel_with_idle_timeout(&server, &closed, timeout);

    let result = blaze_remote::runtime::block_on(
        channel.client_streaming::<WriteRequest, WriteResponse, _>(
            methods::WRITE,
            futures::stream::iter(vec![WriteRequest::default()]),
            CallOptions::new(),
        ),
    );

    assert_eq!(result.unwrap(), WriteResponse { committed_size: 0 });
    assert_eq!(
        closed.get(),
        vec![(methods::WRITE, tonic::Code::Ok, String::new())]
    );
}

#[test]
fn byte_stream_read_cancels_when_idle_timeout_fires() {
    // A ByteStream.Read with no response activity for the full timeout should be canceled so the
    // cache client can retry it rather than waiting forever.
    //
    // PORT: Java uses a 3s timeout on a fake clock and asserts the scheduled delay
    // (`delaysNanos == [3s]`); here the timeout is 1s of real time and the scheduler is not
    // observable.
    let timeout = Duration::from_secs(1);
    let cancelled = Arc::new(AtomicBool::new(false));
    let mut server = FakeServer::new();
    {
        let cancelled = cancelled.clone();
        server.service_registry.add_byte_stream(FakeByteStream {
            read: Some(Box::new(move |_: tonic::Request<ReadRequest>| {
                hanging_response_stream(vec![], cancelled.clone())
            })),
            ..Default::default()
        });
    }
    server.start();
    let closed = ClosedStatuses::default();
    let channel = channel_with_idle_timeout(&server, &closed, timeout);

    let (result, server_cancelled) = blaze_remote::runtime::block_on(async {
        let result = read_all(&channel).await;
        (result, await_cancelled(&cancelled).await)
    });

    let status = result.unwrap_err();
    assert!(server_cancelled, "the Read call was not cancelled");
    assert_eq!(
        status.message(),
        "Remote download idle timeout exceeded after PT1S for google.bytestream.ByteStream/Read"
    );
    assert_eq!(status.code(), tonic::Code::DeadlineExceeded);
    // ClientCall.cancel normally closes the listener with CANCELLED. The interceptor translates
    // its own timeout-driven close so retry classification sees DEADLINE_EXCEEDED instead.
    let closed = closed.get();
    assert_eq!(closed.len(), 1, "{closed:?}");
    assert_eq!(closed[0].0, methods::READ);
    assert_eq!(closed[0].1, tonic::Code::DeadlineExceeded);
}

#[test]
#[ignore = "scheduler internals: Rust's idle timeout has no ScheduledExecutorService, ScheduledFuture or nanoTime supplier to inject, so the task count, the cancelled successor and the scheduled delays (3s, 2s) cannot be observed; the observable part (activity defers the timeout) is byte_stream_read_activity_defers_timeout"]
fn byte_stream_read_defers_timeout_without_queueing_tasks_for_each_message() {
    // Activity should only move the monotonic deadline. Replacing the scheduled task for every
    // message would retain canceled tasks in the shared scheduler and grow its queue.
    //
    // Java body (not expressible): one task scheduled at start with delay 3s; a request at t=1
    // and a response at t=2 leave it the only task; when it runs at t=3 it schedules exactly one
    // successor with delay 2s; onClose(OK) cancels the successor and not the first task;
    // the response reaches the listener and the call is not cancelled.
    unreachable!("ignored: needs an injectable scheduler and clock");
}

#[test]
#[ignore = "scheduler internals: needs a ScheduledExecutorService whose schedule() throws; Rust's idle timeout is tokio::time::timeout and has no scheduler to inject"]
fn byte_stream_read_forwards_callbacks_when_reset_timeout_fails() {
    // Timeout bookkeeping is secondary to the gRPC call. Make every attempt to schedule the
    // timeout fail, then verify the request and response still cross the interceptor boundary.
    //
    // Java body (not expressible): start(), sendMessage(request) and onMessage(response) each
    // throw the scheduler's RuntimeException("reset failed"), yet the delegate received
    // [request] and the listener received [response].
    unreachable!("ignored: needs a failing scheduler");
}

// not a Java test
#[test]
fn byte_stream_read_cancels_when_stalled_after_first_chunk() {
    // The idle timeout runs between messages too: one chunk, then no activity.
    let timeout = Duration::from_secs(1);
    let cancelled = Arc::new(AtomicBool::new(false));
    let mut server = FakeServer::new();
    {
        let cancelled = cancelled.clone();
        server.service_registry.add_byte_stream(FakeByteStream {
            read: Some(Box::new(move |_: tonic::Request<ReadRequest>| {
                hanging_response_stream(vec![Ok(read_response("abc"))], cancelled.clone())
            })),
            ..Default::default()
        });
    }
    server.start();
    let closed = ClosedStatuses::default();
    let channel = channel_with_idle_timeout(&server, &closed, timeout);

    let (first, rest, server_cancelled) = blaze_remote::runtime::block_on(async {
        let mut stream = channel
            .server_streaming::<ReadRequest, ReadResponse>(
                methods::READ,
                ReadRequest::default(),
                CallOptions::new(),
            )
            .await
            .unwrap();
        let first = stream.message().await;
        let rest = stream.message().await;
        drop(stream);
        (first, rest, await_cancelled(&cancelled).await)
    });

    assert_eq!(first.unwrap(), Some(read_response("abc")));
    let status = rest.unwrap_err();
    assert_eq!(status.code(), tonic::Code::DeadlineExceeded);
    assert_eq!(
        status.message(),
        "Remote download idle timeout exceeded after PT1S for google.bytestream.ByteStream/Read"
    );
    assert!(server_cancelled, "the Read call was not cancelled");
    let closed = closed.get();
    assert_eq!(closed.len(), 1, "{closed:?}");
    assert_eq!(closed[0].1, tonic::Code::DeadlineExceeded);
}

// not a Java test
#[test]
fn byte_stream_read_activity_defers_timeout() {
    // The observable half of byteStreamRead_defersTimeoutWithoutQueueingTasksForEachMessage:
    // a Read that keeps sending more often than the idle timeout is not cancelled, even when
    // the whole call takes longer than the timeout.
    let timeout = Duration::from_secs(1);
    let mut server = FakeServer::new();
    server.service_registry.add_byte_stream(FakeByteStream {
        read: Some(Box::new(|_: tonic::Request<ReadRequest>| {
            delayed_stream(
                vec![
                    read_response("a"),
                    read_response("b"),
                    read_response("c"),
                    read_response("d"),
                    read_response("e"),
                ],
                Duration::from_millis(400),
            )
        })),
        ..Default::default()
    });
    server.start();
    let closed = ClosedStatuses::default();
    let channel = channel_with_idle_timeout(&server, &closed, timeout);

    let responses = blaze_remote::runtime::block_on(read_all(&channel)).unwrap();

    assert_eq!(
        responses,
        vec![
            read_response("a"),
            read_response("b"),
            read_response("c"),
            read_response("d"),
            read_response("e"),
        ]
    );
    assert_eq!(
        closed.get(),
        vec![(methods::READ, tonic::Code::Ok, String::new())]
    );
}

// not a Java test
#[test]
fn other_streaming_methods_do_not_get_idle_timeout() {
    // Only ByteStream.Read gets the timeout: a GetTree stream that pauses longer than the idle
    // timeout between messages completes.
    let timeout = Duration::from_millis(500);
    let response = GetTreeResponse {
        next_page_token: "token".to_string(),
        ..Default::default()
    };
    let mut server = FakeServer::new();
    {
        let response = response.clone();
        server
            .service_registry
            .add_content_addressable_storage(FakeContentAddressableStorage {
                get_tree: Some(Box::new(move |_: tonic::Request<GetTreeRequest>| {
                    delayed_stream(vec![response.clone()], timeout * 3)
                })),
                ..Default::default()
            });
    }
    server.start();
    let closed = ClosedStatuses::default();
    let channel = channel_with_idle_timeout(&server, &closed, timeout);

    let responses = blaze_remote::runtime::block_on(async {
        let mut stream = channel
            .server_streaming::<GetTreeRequest, GetTreeResponse>(
                methods::GET_TREE,
                GetTreeRequest {
                    root_digest: Some(Digest::default()),
                    ..Default::default()
                },
                CallOptions::new(),
            )
            .await?;
        let mut responses = Vec::new();
        while let Some(r) = stream.message().await? {
            responses.push(r);
        }
        Ok::<_, tonic::Status>(responses)
    })
    .unwrap();

    assert_eq!(responses, vec![response]);
    assert_eq!(
        closed.get(),
        vec![(methods::GET_TREE, tonic::Code::Ok, String::new())]
    );
}

// ---------------------------------------------------------------------------
// java.time.Duration.toString(), as the idle timeout message uses it
// ---------------------------------------------------------------------------

// not a Java test
#[test]
fn java_duration_to_string_zero() {
    // Duration.ZERO.toString()
    assert_eq!(java_duration_to_string(Duration::ZERO), "PT0S");
}

// not a Java test
#[test]
fn java_duration_to_string_whole_minute() {
    // Duration.ofMinutes(1).toString()
    assert_eq!(java_duration_to_string(Duration::from_secs(60)), "PT1M");
}

// not a Java test
#[test]
fn java_duration_to_string_whole_hour() {
    // Duration.ofHours(1).toString()
    assert_eq!(java_duration_to_string(Duration::from_secs(3600)), "PT1H");
}

// not a Java test
#[test]
fn java_duration_to_string_minute_and_second() {
    // Duration.ofSeconds(61).toString()
    assert_eq!(java_duration_to_string(Duration::from_secs(61)), "PT1M1S");
}

// not a Java test
#[test]
fn java_duration_to_string_fraction_of_second() {
    // Duration.ofMillis(500).toString()
    assert_eq!(
        java_duration_to_string(Duration::from_millis(500)),
        "PT0.5S"
    );
}

// not a Java test
#[test]
fn java_duration_to_string_hours_minutes_seconds_millis() {
    // The JDK's documented example: "PT8H6M12.345S".
    let d = Duration::from_secs(8 * 3600 + 6 * 60 + 12) + Duration::from_millis(345);
    assert_eq!(java_duration_to_string(d), "PT8H6M12.345S");
}
