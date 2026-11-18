#![cfg(feature = "wip")]
// Options and protos are filled in field by field, as the Java tests set them.
#![allow(clippy::field_reassign_with_default)]
//! Port of `src/test/java/com/google/devtools/build/lib/remote/GrpcRemoteExecutorTest.java`.
//!
//! Java runs the tests against an in-process gRPC server; here they run against a fake server
//! on localhost (`util::grpc_server`) that serves a [`FakeExecutionService`]. Java's
//! `assertThrows(IllegalStateException.class, ...)` is a caught panic whose message starts with
//! `IllegalStateException` (`blaze_util::check_state!`).

mod util;

use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use blaze_optdefs::misc::remote_options::RemoteOptions;
use blaze_remote::authandtls::{NO_CREDENTIALS, new_exec_headers_interceptor};
use blaze_remote::common::{
    NoOpOperationObserver, RemoteActionExecutionContext, RemoteExecutionClient,
};
use blaze_remote::execution_status_exception::{self, ExecutionStatusException};
use blaze_remote::grpc::channel::CapabilitiesFetcher;
use blaze_remote::grpc::stubs::execution_server::Execution;
use blaze_remote::grpc::{ClientInterceptor, ReferenceCountedChannel};
use blaze_remote::grpc_remote_executor::GrpcRemoteExecutor;
use blaze_remote::reapi::longrunning::Operation;
use blaze_remote::reapi::rpc;
use blaze_remote::reapi::{
    ActionResult, Digest, ExecuteRequest, ExecuteResponse, ExecutionCapabilities, OutputFile,
    RequestMetadata, ServerCapabilities, WaitExecutionRequest,
};
use blaze_remote::remote_retrier::{ExponentialBackoff, GRPC_RESULT_CLASSIFIER, RemoteRetrier};
use blaze_remote::retrier::{
    ALLOW_ALL_CALLS, Backoff, BackoffSupplier, RejectedExecutionException, ResultClassifier,
    ResultClassifierResult, RetryScheduler,
};
use futures::future::BoxFuture;
use parking_lot::{Condvar, Mutex};
use tonic::{Request, Response, Status};

use util::fake_execution_service::FakeExecutionService;
use util::grpc_server::{FakeServer, ResponseStream};

// ---------------------------------------------------------------------------
// Test fixture fields
// ---------------------------------------------------------------------------

const MAX_RETRY_ATTEMPTS: i32 = 5;

// ---------------------------------------------------------------------------
// Test constants
// ---------------------------------------------------------------------------

fn dummy_output() -> OutputFile {
    OutputFile {
        path: "dummy.txt".to_string(),
        digest: Some(Digest {
            hash: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string(),
            size_bytes: 0,
        }),
        ..Default::default()
    }
}

fn dummy_request() -> ExecuteRequest {
    ExecuteRequest {
        instance_name: "dummy".to_string(),
        action_digest: Some(Digest {
            hash: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string(),
            size_bytes: 0,
        }),
        ..Default::default()
    }
}

fn dummy_response() -> ExecuteResponse {
    ExecuteResponse {
        result: Some(ActionResult {
            output_files: vec![dummy_output()],
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// `TestUtils.ZeroDelayListeningScheduledExecutorService`: schedules all tasks with zero delay.
// Java: com/google/devtools/build/lib/remote/util/TestUtils.java:69 TestUtils.ZeroDelayListeningScheduledExecutorService
struct ZeroDelayRetryScheduler;

#[async_trait]
impl RetryScheduler for ZeroDelayRetryScheduler {
    async fn schedule(&self, _delay_millis: i64) -> Result<(), RejectedExecutionException> {
        Ok(())
    }
}

/// `TestUtils.newRemoteRetrier(backoff, resultClassifier, retryScheduler)`.
// Java: com/google/devtools/build/lib/remote/util/TestUtils.java:46 TestUtils.newRemoteRetrier
fn new_remote_retrier(
    backoff: BackoffSupplier,
    result_classifier: Arc<dyn ResultClassifier>,
) -> Arc<RemoteRetrier> {
    Arc::new(RemoteRetrier::with_sleeper(
        backoff,
        Arc::new(move |e: &io::Error| {
            // `Status.fromThrowable(e)`: an ExecutionStatusException is a
            // StatusRuntimeException in Java.
            if execution_status_exception::status_from_throwable(e).map(|s| s.code())
                == Some(tonic::Code::Cancelled)
            {
                ResultClassifierResult::Success
            } else {
                result_classifier.test(e)
            }
        }),
        Arc::new(ZeroDelayRetryScheduler),
        ALLOW_ALL_CALLS.clone(),
        Arc::new(|_millis: i64| -> io::Result<()> {
            /* don't wait in tests */
            Ok(())
        }),
    ))
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

struct Fixture {
    context: RemoteActionExecutionContext<'static>,
    execution_service: FakeExecutionService,
    remote_options: RemoteOptions,
    fake_server: FakeServer,
    executor: Arc<dyn RemoteExecutionClient>,
}

/// The `ChannelConnectionWithServerCapabilitiesFactory` of the tests: execution enabled.
fn capabilities_fetcher() -> CapabilitiesFetcher {
    Arc::new(
        |_channel: ReferenceCountedChannel| -> BoxFuture<'static, io::Result<ServerCapabilities>> {
            Box::pin(async {
                Ok(ServerCapabilities {
                    execution_capabilities: Some(ExecutionCapabilities {
                        exec_enabled: true,
                        ..Default::default()
                    }),
                    ..Default::default()
                })
            })
        },
    )
}

impl Fixture {
    // Java: setUp
    fn set_up() -> Fixture {
        let context = RemoteActionExecutionContext::create(RequestMetadata::default());

        let execution_service = FakeExecutionService::new();

        let mut fake_server = FakeServer::new();
        fake_server
            .service_registry
            .add_execution(execution_service.clone());
        fake_server.start();

        let mut remote_options = RemoteOptions::default();
        remote_options.remote_max_retry_attempts = MAX_RETRY_ATTEMPTS;

        // InProcessChannelBuilder.forName(fakeServerName)
        //     .intercept(TracingMetadataUtils.newExecHeadersInterceptor(remoteOptions))
        let interceptors: Vec<Arc<dyn ClientInterceptor>> =
            vec![Arc::new(new_exec_headers_interceptor(&remote_options))];
        let channel = fake_server.channel(interceptors, Some(capabilities_fetcher()));

        let executor = create_execution_service(&remote_options, channel);
        Fixture {
            context,
            execution_service,
            remote_options,
            fake_server,
            executor,
        }
    }

    fn execute_remotely(&self) -> io::Result<ExecuteResponse> {
        self.executor
            .execute_remotely(&self.context, dummy_request(), &NoOpOperationObserver)
    }
}

impl Drop for Fixture {
    // Java: tearDown
    fn drop(&mut self) {
        self.fake_server.shutdown_now();

        self.executor.close();
    }
}

fn create_execution_service(
    remote_options: &RemoteOptions,
    channel: ReferenceCountedChannel,
) -> Arc<dyn RemoteExecutionClient> {
    let remote_options = remote_options.clone();
    let retrier = new_remote_retrier(
        Arc::new(move || -> Arc<dyn Backoff> {
            Arc::new(ExponentialBackoff::from_options(&remote_options))
        }),
        GRPC_RESULT_CLASSIFIER.clone(),
    );

    Arc::new(GrpcRemoteExecutor::new(
        channel,
        NO_CREDENTIALS.clone(),
        retrier,
    ))
}

/// `assertThrows(IllegalStateException.class, f)`: returns the exception's message.
fn assert_throws_illegal_state<T>(f: impl FnOnce() -> T) -> String {
    let payload = match catch_unwind(AssertUnwindSafe(f)) {
        Ok(_) => panic!("expected an IllegalStateException"),
        Err(payload) => payload,
    };
    let message = if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = payload.downcast_ref::<&str>() {
        s.to_string()
    } else {
        panic!("expected an IllegalStateException, got a panic without a message")
    };
    assert!(
        message.starts_with("IllegalStateException"),
        "expected an IllegalStateException, got: {message}"
    );
    message
}

/// `Status.X.asRuntimeException()`.
fn status(code: tonic::Code) -> Status {
    Status::new(code, "")
}

#[test]
fn execute_remotely_smoke() {
    let t = Fixture::set_up();
    t.execution_service
        .when_execute(&dummy_request())
        .then_ack()
        .then_ack()
        .then_done(&dummy_response());

    let response = t.execute_remotely().unwrap();

    assert_eq!(response, dummy_response());
    assert_eq!(t.execution_service.get_exec_times(), 1);
}

#[test]
fn execute_remotely_error_in_operation_retry_execute() {
    let t = Fixture::set_up();
    // `thenError(new RuntimeException("Unavailable"))`: the client sees
    // `Status.fromThrowable(e)`, which is UNKNOWN.
    t.execution_service
        .when_execute(&dummy_request())
        .then_error_exception(status(tonic::Code::Unknown));
    t.execution_service
        .when_execute(&dummy_request())
        .then_error(tonic::Code::Unavailable);
    t.execution_service
        .when_execute(&dummy_request())
        .then_ack()
        .then_done(&dummy_response());

    let response = t.execute_remotely().unwrap();

    assert_eq!(t.execution_service.get_exec_times(), 3);
    assert_eq!(response, dummy_response());
}

#[test]
fn execute_remotely_error_in_response_retry_execute() {
    let t = Fixture::set_up();
    t.execution_service
        .when_execute(&dummy_request())
        .then_done(&ExecuteResponse {
            status: Some(rpc::Status {
                code: tonic::Code::Unavailable as i32,
                ..Default::default()
            }),
            ..Default::default()
        });
    t.execution_service
        .when_execute(&dummy_request())
        .then_ack()
        .then_done(&dummy_response());

    let response = t.execute_remotely().unwrap();

    assert_eq!(t.execution_service.get_exec_times(), 2);
    assert_eq!(response, dummy_response());
}

#[test]
fn execute_remotely_unretriable_error_in_response_report_error() {
    let t = Fixture::set_up();
    t.execution_service
        .when_execute(&dummy_request())
        .then_done(&ExecuteResponse {
            status: Some(rpc::Status {
                code: tonic::Code::InvalidArgument as i32,
                ..Default::default()
            }),
            ..Default::default()
        });
    t.execution_service
        .when_execute(&dummy_request())
        .then_ack()
        .then_done(&dummy_response());

    let e = t.execute_remotely().unwrap_err();

    assert!(e.to_string().contains("INVALID_ARGUMENT"), "message: {}", e);
    assert_eq!(t.execution_service.get_exec_times(), 1);
}

#[test]
fn execute_remotely_retry_execute_and_fail() {
    let t = Fixture::set_up();
    for _ in 0..=MAX_RETRY_ATTEMPTS * 2 {
        t.execution_service
            .when_execute(&dummy_request())
            .then_error(tonic::Code::Unavailable);
    }

    let exception = t.execute_remotely().unwrap_err();

    assert_eq!(t.execution_service.get_exec_times(), MAX_RETRY_ATTEMPTS + 1);
    assert!(
        exception.to_string().contains("UNAVAILABLE"),
        "message: {}",
        exception
    );
}

#[test]
fn execute_remotely_execute_and_wait() {
    let t = Fixture::set_up();
    t.execution_service
        .when_execute(&dummy_request())
        .then_ack()
        .finish();
    t.execution_service
        .when_wait_execution(&dummy_request())
        .then_done(&dummy_response());

    let response = t.execute_remotely().unwrap();

    assert_eq!(t.execution_service.get_exec_times(), 1);
    assert_eq!(t.execution_service.get_wait_times(), 1);
    assert_eq!(response, dummy_response());
}

#[test]
fn execute_remotely_execute_and_retry_wait() {
    let t = Fixture::set_up();
    t.execution_service
        .when_execute(&dummy_request())
        .then_ack()
        .finish();
    t.execution_service
        .when_wait_execution(&dummy_request())
        .then_done(&dummy_response());

    let response = t.execute_remotely().unwrap();

    assert_eq!(t.execution_service.get_exec_times(), 1);
    assert_eq!(t.execution_service.get_wait_times(), 1);
    assert_eq!(response, dummy_response());
}

#[test]
fn execute_remotely_retry_execute_when_unauthenticated() {
    let t = Fixture::set_up();
    t.execution_service
        .when_execute(&dummy_request())
        .then_error(tonic::Code::Unauthenticated);
    t.execution_service
        .when_execute(&dummy_request())
        .then_ack()
        .then_done(&dummy_response());

    let response = t.execute_remotely().unwrap();

    assert_eq!(t.execution_service.get_exec_times(), 2);
    assert_eq!(response, dummy_response());
}

#[test]
fn execute_remotely_retry_wait_execution_when_unauthenticated_error_runtime_exception() {
    let t = Fixture::set_up();
    // This test corresponds to the one in the original base class that used a
    // Status runtime exception while waiting.
    t.execution_service
        .when_execute(&dummy_request())
        .then_ack()
        .finish();
    t.execution_service
        .when_wait_execution(&dummy_request())
        .then_error_exception(status(tonic::Code::Unauthenticated));
    t.execution_service
        .when_wait_execution(&dummy_request())
        .then_ack()
        .then_done(&dummy_response());

    let response = t.execute_remotely().unwrap();

    assert_eq!(t.execution_service.get_exec_times(), 1);
    assert_eq!(t.execution_service.get_wait_times(), 2);
    assert_eq!(response, dummy_response());
}

#[test]
fn execute_remotely_retry_wait_execution_when_unauthenticated_error_code_unauthenticated() {
    let t = Fixture::set_up();
    // Variant from the former derived test that injected an UNAUTHENTICATED
    // status through the fake execution service using Code.UNAUTHENTICATED.
    t.execution_service
        .when_execute(&dummy_request())
        .then_ack()
        .finish();
    t.execution_service
        .when_wait_execution(&dummy_request())
        .then_error(tonic::Code::Unauthenticated);
    t.execution_service
        .when_execute(&dummy_request())
        .then_ack()
        .then_done(&dummy_response());

    let response = t.execute_remotely().unwrap();

    assert_eq!(t.execution_service.get_exec_times(), 2);
    assert_eq!(t.execution_service.get_wait_times(), 1);
    assert_eq!(response, dummy_response());
}

#[test]
fn execute_remotely_operation_without_result_crashes() {
    let t = Fixture::set_up();
    t.execution_service
        .when_execute(&dummy_request())
        .then_done_without_response();

    assert_throws_illegal_state(|| t.execute_remotely());

    // Shouldn't retry in this case
    assert_eq!(t.execution_service.get_exec_times(), 1);
}

#[test]
fn execute_remotely_response_without_result_crashes() {
    let t = Fixture::set_up();
    t.execution_service
        .when_execute(&dummy_request())
        .then_done(&ExecuteResponse::default());

    assert_throws_illegal_state(|| t.execute_remotely());

    assert_eq!(t.execution_service.get_exec_times(), 1);
}

#[test]
fn execute_remotely_operation_without_result_should_not_crash() {
    let t = Fixture::set_up();
    t.execution_service
        .when_execute(&dummy_request())
        .then_done_without_response();

    let message = assert_throws_illegal_state(|| t.execute_remotely());
    assert!(
        message.contains("Unexpected result of remote execution: result not set"),
        "message: {message}"
    );

    // Shouldn't retry in this case
    assert_eq!(t.execution_service.get_exec_times(), 1);
}

#[test]
fn execute_remotely_response_without_result_should_not_retry() {
    let t = Fixture::set_up();
    t.execution_service
        .when_execute(&dummy_request())
        .then_done(&ExecuteResponse::default());

    let message = assert_throws_illegal_state(|| t.execute_remotely());
    assert!(
        message.contains("Unexpected result of remote execution: no result"),
        "message: {message}"
    );

    // Shouldn't retry in this case
    assert_eq!(t.execution_service.get_exec_times(), 1);
}

#[test]
fn execute_remotely_retry_execute_if_not_found() {
    let t = Fixture::set_up();
    t.execution_service
        .when_execute(&dummy_request())
        .then_ack()
        .finish();
    t.execution_service
        .when_wait_execution(&dummy_request())
        .then_error(tonic::Code::NotFound);
    t.execution_service
        .when_execute(&dummy_request())
        .then_ack()
        .finish();
    t.execution_service
        .when_wait_execution(&dummy_request())
        .then_done(&dummy_response());

    let response = t.execute_remotely().unwrap();

    assert_eq!(t.execution_service.get_exec_times(), 2);
    assert_eq!(t.execution_service.get_wait_times(), 2);
    assert_eq!(response, dummy_response());
}

#[test]
fn execute_remotely_retry_execute_if_not_found_stream() {
    let t = Fixture::set_up();
    t.execution_service
        .when_execute(&dummy_request())
        .then_ack()
        .finish();
    t.execution_service
        .when_wait_execution(&dummy_request())
        .then_error_exception(status(tonic::Code::NotFound));
    t.execution_service
        .when_execute(&dummy_request())
        .then_ack()
        .finish();
    t.execution_service
        .when_wait_execution(&dummy_request())
        .then_done(&dummy_response());

    let response = t.execute_remotely().unwrap();

    assert_eq!(t.execution_service.get_exec_times(), 2);
    assert_eq!(t.execution_service.get_wait_times(), 2);
    assert_eq!(response, dummy_response());
}

#[test]
fn execute_remotely_retry_execute_on_finish() {
    let t = Fixture::set_up();
    t.execution_service
        .when_execute(&dummy_request())
        .then_ack()
        .finish();
    t.execution_service
        .when_wait_execution(&dummy_request())
        .then_ack()
        .finish();
    t.execution_service
        .when_wait_execution(&dummy_request())
        .then_ack()
        .then_done(&dummy_response());

    let response = t.execute_remotely().unwrap();

    assert_eq!(t.execution_service.get_exec_times(), 1);
    assert_eq!(t.execution_service.get_wait_times(), 2);
    assert_eq!(response, dummy_response());
}

#[test]
fn execute_remotely_not_found_loop_report_error() {
    let t = Fixture::set_up();
    for _ in 0..=MAX_RETRY_ATTEMPTS {
        t.execution_service
            .when_execute(&dummy_request())
            .then_ack()
            .finish();
        t.execution_service
            .when_wait_execution(&dummy_request())
            .then_error(tonic::Code::NotFound);
    }

    let e = t.execute_remotely().unwrap_err();

    // assertThat(e).hasCauseThat().isInstanceOf(ExecutionStatusException.class)
    let execution_status_exception =
        blaze_remote::common::errors::downcast_ref::<ExecutionStatusException>(&e)
            .unwrap_or_else(|| panic!("cause is not an ExecutionStatusException: {e}"));
    assert_eq!(
        execution_status_exception.get_status().code(),
        tonic::Code::NotFound
    );
    assert_eq!(t.execution_service.get_exec_times(), MAX_RETRY_ATTEMPTS + 1);
    assert_eq!(t.execution_service.get_wait_times(), MAX_RETRY_ATTEMPTS + 1);
}

#[test]
fn execute_remotely_notify_observer() {
    let t = Fixture::set_up();
    t.execution_service
        .when_execute(&dummy_request())
        .then_ack()
        .then_done(&dummy_response());

    let notified: Mutex<Vec<Operation>> = Mutex::new(Vec::new());
    let observer = |o: &Operation| -> io::Result<()> {
        notified.lock().push(o.clone());
        Ok(())
    };
    let _unused = t
        .executor
        .execute_remotely(&t.context, dummy_request(), &observer)
        .unwrap();

    assert_eq!(
        *notified.lock(),
        vec![
            FakeExecutionService::ack_operation(&dummy_request()),
            FakeExecutionService::done_operation(&dummy_request(), &dummy_response()),
        ]
    );
}

#[test]
fn execute_remotely_retry_execute_on_no_result_done_operation() {
    let t = Fixture::set_up();
    t.execution_service
        .when_execute(&dummy_request())
        .then_error(tonic::Code::Unavailable);
    t.execution_service
        .when_execute(&dummy_request())
        .then_ack()
        .then_done(&dummy_response());

    let response = t.execute_remotely().unwrap();

    assert_eq!(t.execution_service.get_exec_times(), 2);
    assert_eq!(t.execution_service.get_wait_times(), 0);
    assert_eq!(response, dummy_response());
}

#[test]
fn execute_remotely_execute_and_retry_wait_forever() {
    let t = Fixture::set_up();
    t.execution_service
        .when_execute(&dummy_request())
        .then_ack()
        .finish();
    let error_times = MAX_RETRY_ATTEMPTS;
    for _ in 0..error_times {
        t.execution_service
            .when_wait_execution(&dummy_request())
            .then_error_exception(status(tonic::Code::DeadlineExceeded));
    }
    t.execution_service
        .when_wait_execution(&dummy_request())
        .then_done(&dummy_response());

    let response = t.execute_remotely().unwrap();

    assert_eq!(t.execution_service.get_exec_times(), 1);
    assert_eq!(t.execution_service.get_wait_times(), error_times + 1);
    assert_eq!(response, dummy_response());
}

#[test]
fn execute_remotely_execute_and_retry_wait_fail_for_consecutive_errors() {
    let t = Fixture::set_up();
    t.execution_service
        .when_execute(&dummy_request())
        .then_ack()
        .finish();
    for _ in 0..MAX_RETRY_ATTEMPTS * 2 {
        t.execution_service
            .when_wait_execution(&dummy_request())
            .then_error_exception(status(tonic::Code::Unavailable));
    }

    assert!(t.execute_remotely().is_err());

    assert_eq!(t.execution_service.get_exec_times(), 1);
    assert_eq!(t.execution_service.get_wait_times(), MAX_RETRY_ATTEMPTS + 1);
}

/// `java.util.concurrent.CountDownLatch` with a count of one, awaitable from a test thread
/// (`await(timeout)`) and from a server task.
struct CountDownLatch {
    done: Mutex<bool>,
    condvar: Condvar,
    notify: tokio::sync::Notify,
}

impl CountDownLatch {
    fn new() -> CountDownLatch {
        CountDownLatch {
            done: Mutex::new(false),
            condvar: Condvar::new(),
            notify: tokio::sync::Notify::new(),
        }
    }

    fn count_down(&self) {
        *self.done.lock() = true;
        self.condvar.notify_all();
        self.notify.notify_waiters();
    }

    /// `await(timeout, unit)`.
    fn await_timeout(&self, timeout: Duration) -> bool {
        let mut done = self.done.lock();
        if !*done {
            let _ = self.condvar.wait_for(&mut done, timeout);
        }
        *done
    }

    /// `await()` from a server task.
    async fn wait(&self) {
        loop {
            let notified = self.notify.notified();
            if *self.done.lock() {
                return;
            }
            notified.await;
        }
    }
}

#[test]
#[ignore = "needs a connection pool: the port's ReferenceCountedChannel multiplexes one HTTP/2 \
            connection (no maxConcurrency, no channel creations to count)"]
fn execute_remotely_holds_channel_lease_until_execute_stream_completes() {
    let t = Fixture::set_up();
    let blocking_execution_service = BlockingExecutionService::new();
    let mut server = FakeServer::new();
    server
        .service_registry
        .add_execution(blocking_execution_service.clone());
    server.start();
    let channel_creations = Arc::new(AtomicI32::new(0));
    // NEEDS(channel): a factory with maxConcurrency() == 1 that counts its channel creations;
    // the capabilities fetcher stands in for the factory's `create()`.
    let fetcher: CapabilitiesFetcher = {
        let channel_creations = channel_creations.clone();
        let inner = capabilities_fetcher();
        Arc::new(move |channel: ReferenceCountedChannel| {
            channel_creations.fetch_add(1, Ordering::SeqCst);
            inner(channel)
        })
    };
    let interceptors: Vec<Arc<dyn ClientInterceptor>> =
        vec![Arc::new(new_exec_headers_interceptor(&t.remote_options))];
    let channel = server.channel(interceptors, Some(fetcher));
    let executor = create_execution_service(&t.remote_options, channel);

    let (first_tx, first_rx) = std::sync::mpsc::channel();
    let (second_tx, second_rx) = std::sync::mpsc::channel();
    {
        let executor = executor.clone();
        std::thread::spawn(move || {
            let context = RemoteActionExecutionContext::create(RequestMetadata::default());
            let _ = first_tx.send(executor.execute_remotely(
                &context,
                dummy_request(),
                &NoOpOperationObserver,
            ));
        });
    }
    assert!(
        blocking_execution_service
            .inner
            .first_execute_started
            .await_timeout(Duration::from_secs(1))
    );
    assert_eq!(channel_creations.load(Ordering::SeqCst), 1);

    {
        let executor = executor.clone();
        std::thread::spawn(move || {
            let context = RemoteActionExecutionContext::create(RequestMetadata::default());
            let _ = second_tx.send(executor.execute_remotely(
                &context,
                dummy_request(),
                &NoOpOperationObserver,
            ));
        });
    }
    assert!(
        blocking_execution_service
            .inner
            .second_execute_started
            .await_timeout(Duration::from_secs(1))
    );

    assert_eq!(channel_creations.load(Ordering::SeqCst), 2);

    blocking_execution_service
        .inner
        .complete_first_execute
        .count_down();
    assert_eq!(
        second_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap(),
        dummy_response()
    );
    assert_eq!(
        first_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap(),
        dummy_response()
    );

    // finally
    blocking_execution_service
        .inner
        .complete_first_execute
        .count_down();
    executor.close();
    server.shutdown_now();
}

struct BlockingExecutionServiceInner {
    execute_times: AtomicI32,
    first_execute_started: CountDownLatch,
    second_execute_started: CountDownLatch,
    complete_first_execute: CountDownLatch,
}

#[derive(Clone)]
struct BlockingExecutionService {
    inner: Arc<BlockingExecutionServiceInner>,
}

impl BlockingExecutionService {
    fn new() -> BlockingExecutionService {
        BlockingExecutionService {
            inner: Arc::new(BlockingExecutionServiceInner {
                execute_times: AtomicI32::new(0),
                first_execute_started: CountDownLatch::new(),
                second_execute_started: CountDownLatch::new(),
                complete_first_execute: CountDownLatch::new(),
            }),
        }
    }
}

#[tonic::async_trait]
impl Execution for BlockingExecutionService {
    type ExecuteStream = ResponseStream<Operation>;
    type WaitExecutionStream = ResponseStream<Operation>;

    async fn execute(
        &self,
        request: Request<ExecuteRequest>,
    ) -> Result<Response<Self::ExecuteStream>, Status> {
        use futures::StreamExt;
        let request = request.into_inner();
        let execute_number = self.inner.execute_times.fetch_add(1, Ordering::SeqCst) + 1;
        let done: Result<Operation, Status> = Ok(FakeExecutionService::done_operation(
            &request,
            &dummy_response(),
        ));
        if execute_number == 1 {
            let ack = FakeExecutionService::ack_operation(&request);
            let inner = self.inner.clone();
            // onNext(ack); firstExecuteStarted.countDown(); completeFirstExecute.await();
            // onNext(done); onCompleted();
            let stream: ResponseStream<Operation> = Box::pin(
                futures::stream::iter(vec![Ok(ack)]).chain(futures::stream::once(async move {
                    inner.first_execute_started.count_down();
                    inner.complete_first_execute.wait().await;
                    done
                })),
            );
            return Ok(Response::new(stream));
        } else if execute_number == 2 {
            self.inner.second_execute_started.count_down();
        }

        let stream: ResponseStream<Operation> = Box::pin(futures::stream::iter(vec![done]));
        Ok(Response::new(stream))
    }

    async fn wait_execution(
        &self,
        _request: Request<WaitExecutionRequest>,
    ) -> Result<Response<Self::WaitExecutionStream>, Status> {
        Err(Status::unimplemented(
            "Method build.bazel.remote.execution.v2.Execution/WaitExecution is unimplemented",
        ))
    }
}
