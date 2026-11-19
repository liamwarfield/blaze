//! Unit tests for the gRPC server.
//!
//! Java: src/test/java/com/google/devtools/build/lib/server/GrpcServerTest.java
//!
//! PORT: Java connects grpc-java's in-process transport; these tests serve the same
//! `GrpcServerImpl` on loopback TCP (`grpc::Server`, tonic) and call it with tonic's client.
//! Java's response observers are reads of the response stream. The flow-control tests give the
//! client a small HTTP/2 stream window ([`FLOW_CONTROL_WINDOW`]) so that the server's sends
//! block after a few messages, as grpc-java's in-process transport does (its buffer is what the
//! Java comments call "gRPC's internal buffers").

use std::sync::Arc;
use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};
use std::time::Duration;

use blaze_events::io::OutErr;
use blaze_events::ui::JavaClock;
use blaze_protos::blaze::invocation_policy::InvocationPolicy;
use blaze_protos::command_server::{
    CancelRequest, CancelResponse, PingRequest, PingResponse, RunRequest, RunResponse,
    TerminalSizeRequest, TerminalSizeResponse,
};
use blaze_protos::failure_details::failure_detail::Category;
use blaze_protos::failure_details::{
    FailureDetail, GrpcServer, Interrupted, command, grpc_server, interrupted,
};
use blaze_protos::google::protobuf::Any;
use blaze_server::command_dispatcher::IdleTaskResultsSupplier;
use blaze_server::grpc::Server;
use blaze_server::grpc::stubs::command_server_client::CommandServerClient;
use blaze_server::grpc::stubs::command_server_server::CommandServer;
use blaze_server::grpc_server_impl::{BlockingStreamObserver, CommandServerRunStream};
use blaze_server::interruptible::{self, Monitor};
use blaze_server::{
    BlazeCommandResult, CommandDispatcher, CommandExtensionReporter, GrpcServerImpl, IdleTask,
    IdleTaskError, IdleTaskResult, IdleTaskStatus, LockingMode, PidFileWatcher, ShutdownHooks,
    TerminalSizeMonitor, UiVerbosity,
};
use blaze_shell::thread_interrupt::{self, InterruptedException};
use blaze_vfs::{DigestHashFunction, Path, UnixFileSystem};
use parking_lot::{Condvar, Mutex};
use prost::Message;
use tonic::transport::{Channel, Endpoint};
use tonic::{Request, Response, Status, Streaming};

const SERVER_PID: i32 = 42;
const REQUEST_COOKIE: &str = "request-cookie";
/// `TestUtils.WAIT_TIMEOUT_SECONDS`.
const WAIT_TIMEOUT: Duration = Duration::from_secs(60);
/// The client's HTTP/2 stream window in the flow-control tests (see the module comment).
const FLOW_CONTROL_WINDOW: u32 = 1024;

// ------------------------------------------------------------------------------------------
// Test plumbing: latches, dispatchers, server and client.

/// `java.util.concurrent.CountDownLatch`.
struct CountDownLatch {
    count: Mutex<usize>,
    cond: Condvar,
}

impl CountDownLatch {
    fn new(count: usize) -> Arc<Self> {
        Arc::new(CountDownLatch {
            count: Mutex::new(count),
            cond: Condvar::new(),
        })
    }

    fn count_down(&self) {
        let mut count = self.count.lock();
        if *count > 0 {
            *count -= 1;
        }
        self.cond.notify_all();
    }

    /// `await(timeout)`: whether the count reached zero.
    fn await_timeout(&self, timeout: Duration) -> bool {
        let mut count = self.count.lock();
        let deadline = std::time::Instant::now() + timeout;
        while *count > 0 {
            if self.cond.wait_until(&mut count, deadline).timed_out() {
                return *count == 0;
            }
        }
        true
    }

    fn await_(&self) {
        assert!(self.await_timeout(WAIT_TIMEOUT), "latch timed out");
    }
}

type ExecFn = dyn Fn(
        Vec<String>,
        OutErr,
        IdleTaskResultsSupplier,
        Vec<Any>,
        Arc<dyn CommandExtensionReporter>,
    ) -> Result<BlazeCommandResult, InterruptedException>
    + Send
    + Sync;

/// A `CommandDispatcher` whose `exec` is a closure.
struct FnDispatcher(Box<ExecFn>);

impl CommandDispatcher for FnDispatcher {
    fn exec(
        &self,
        _invocation_policy: InvocationPolicy,
        args: Vec<String>,
        out_err: OutErr,
        _locking_mode: LockingMode,
        _ui_verbosity: UiVerbosity,
        _client_description: String,
        _first_contact_time_millis: i64,
        _startup_options_tagged_with_bazel_rc: Option<Vec<(String, String)>>,
        idle_task_results_supplier: IdleTaskResultsSupplier,
        command_extensions: Vec<Any>,
        command_extension_reporter: Arc<dyn CommandExtensionReporter>,
    ) -> Result<BlazeCommandResult, InterruptedException> {
        (self.0)(
            args,
            out_err,
            idle_task_results_supplier,
            command_extensions,
            command_extension_reporter,
        )
    }
}

fn dispatcher(
    f: impl Fn(
        Vec<String>,
        OutErr,
        IdleTaskResultsSupplier,
        Vec<Any>,
        Arc<dyn CommandExtensionReporter>,
    ) -> Result<BlazeCommandResult, InterruptedException>
    + Send
    + Sync
    + 'static,
) -> Arc<dyn CommandDispatcher> {
    Arc::new(FnDispatcher(Box::new(f)))
}

/// The server, a client runtime and a channel (Java's `server` and `channel` fields).
struct Fixture {
    _dir: tempfile::TempDir,
    server: Server,
    runtime: tokio::runtime::Runtime,
    port: u16,
}

impl Fixture {
    fn client(&self) -> CommandServerClient<Channel> {
        self.client_with_window(None)
    }

    fn client_with_window(&self, window: Option<u32>) -> CommandServerClient<Channel> {
        let endpoint = Endpoint::from_shared(format!("http://127.0.0.1:{}", self.port))
            .unwrap()
            .initial_stream_window_size(window)
            .initial_connection_window_size(window);
        let channel = self.runtime.block_on(endpoint.connect()).unwrap();
        CommandServerClient::new(channel)
    }

    fn block_on<F: std::future::Future>(&self, f: F) -> F::Output {
        self.runtime.block_on(f)
    }

    fn spawn<F>(&self, f: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: std::future::Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.runtime.spawn(f)
    }

    /// `server.shutdown(); server.awaitTermination();`
    fn shutdown_and_await(&self) {
        self.server.shutdown();
        self.server.await_termination();
    }
}

fn start_server(service: impl CommandServer, dir: tempfile::TempDir) -> Fixture {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let server = Server::start(listener, service).unwrap();
    let port = server.get_port();
    Fixture {
        _dir: dir,
        server,
        runtime: tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap(),
        port,
    }
}

// Java: GrpcServerTest.createServer
fn create_server(dispatcher: Arc<dyn CommandDispatcher>) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
    let root = dir.path().to_str().unwrap().to_string();
    let server_directory = Path::create_from_str(&format!("{root}/bazel_server_directory"), &fs);
    server_directory.create_directory_and_parents().unwrap();

    let server_impl = GrpcServerImpl::new(
        dispatcher,
        ShutdownHooks::create_unregistered(),
        Arc::new(PidFileWatcher::new(
            Path::create_from_str(&format!("{root}/thread-not-running-dont-need"), &fs),
            SERVER_PID,
        )),
        Arc::new(JavaClock),
        /* port= */ -1,
        REQUEST_COOKIE.to_string(),
        "response-cookie".to_string(),
        server_directory,
        SERVER_PID,
        /* max_idle_seconds= */ 1000,
        /* shutdown_on_low_sys_mem= */ false,
        /* do_idle_server_tasks= */ true,
        Some("slow interrupt message suffix".to_string()),
    );
    start_server(server_impl.service(), dir)
}

fn create_request(args: &[&str]) -> RunRequest {
    RunRequest {
        cookie: REQUEST_COOKIE.to_string(),
        client_description: "client-description".to_string(),
        arg: args.iter().map(|a| a.as_bytes().to_vec()).collect(),
        ..Default::default()
    }
}

fn create_preemptible_request(args: &[&str]) -> RunRequest {
    RunRequest {
        preemptible: true,
        ..create_request(args)
    }
}

/// `Any.pack(message)`.
fn pack<M: Message>(type_name: &str, message: &M) -> Any {
    Any {
        type_url: format!("type.googleapis.com/{type_name}"),
        value: message.encode_to_vec(),
    }
}

fn int32_value(v: i32) -> Any {
    // Int32Value.of(v): prost encodes `i32` as google.protobuf.Int32Value.
    pack("google.protobuf.Int32Value", &v)
}

/// Reads the whole response stream (Java's `createResponseObserver` until onCompleted/onError).
async fn collect(mut stream: Streaming<RunResponse>) -> Vec<RunResponse> {
    let mut responses = Vec::new();
    while let Ok(Some(response)) = stream.message().await {
        responses.push(response);
    }
    responses
}

async fn run(
    client: &mut CommandServerClient<Channel>,
    request: RunRequest,
) -> Streaming<RunResponse> {
    client.run(request).await.unwrap().into_inner()
}

fn interrupted_failure_detail(code: interrupted::Code) -> FailureDetail {
    FailureDetail {
        message: String::new(),
        category: Some(Category::Interrupted(Interrupted { code: code as i32 })),
    }
}

/// `throwingDispatcher()`.
fn throwing_dispatcher() -> Arc<dyn CommandDispatcher> {
    dispatcher(|_, _, _, _, _| panic!("IllegalStateException: Command exec not expected"))
}

// ------------------------------------------------------------------------------------------

#[test]
fn test_sending_simple_message() {
    // Arbitrary message.
    let command_extension = pack(
        "command_server.EnvironmentVariable",
        &blaze_protos::command_server::EnvironmentVariable::default(),
    );
    let args_received = Arc::new(Mutex::new(None));
    let command_extensions_received = Arc::new(Mutex::new(None));
    let (a, c) = (args_received.clone(), command_extensions_received.clone());
    let fixture = create_server(dispatcher(move |args, _, _, exts, _| {
        *a.lock() = Some(args);
        *c.lock() = Some(exts);
        Ok(BlazeCommandResult::success())
    }));

    let mut stub = fixture.client();
    let mut request = create_request(&["Foo"]);
    request.command_extensions.push(command_extension.clone());
    let responses = fixture.block_on(async { collect(run(&mut stub, request).await).await });
    drop(stub);
    fixture.shutdown_and_await();

    assert_eq!(
        args_received.lock().clone().unwrap(),
        vec!["Foo".to_string()]
    );
    assert_eq!(
        command_extensions_received.lock().clone().unwrap(),
        vec![command_extension]
    );

    assert_eq!(responses.len(), 2);
    assert!(!responses[0].finished);
    assert!(!responses[0].cookie.is_empty());
    assert!(responses[1].finished);
    assert_eq!(responses[1].exit_code, 0);
    assert!(responses[1].failure_detail.is_none());
}

#[test]
fn test_receive_streaming_command_extensions() {
    // Arrange: Set up a command that streams back three command extensions, using latches to
    // pause between each extension sent back.
    let command_extension1 = int32_value(4);
    let command_extension2 = int32_value(8);
    let command_extension3 = int32_value(15);

    let after_first_extension_latch = CountDownLatch::new(1);
    let before_second_extension_latch = CountDownLatch::new(1);
    let after_second_extension_latch = CountDownLatch::new(1);
    let before_third_extension_latch = CountDownLatch::new(1);
    let after_third_extension_latch = CountDownLatch::new(1);
    let (e1, e2, e3) = (
        command_extension1.clone(),
        command_extension2.clone(),
        command_extension3.clone(),
    );
    let (l1, l2, l3, l4, l5) = (
        after_first_extension_latch.clone(),
        before_second_extension_latch.clone(),
        after_second_extension_latch.clone(),
        before_third_extension_latch.clone(),
        after_third_extension_latch.clone(),
    );
    let fixture = create_server(dispatcher(move |_, _, _, _, cmd_ext_out| {
        // Send the first extension.
        cmd_ext_out.report(e1.clone());
        l1.count_down();
        // Send the second extension.
        l2.await_timeout(WAIT_TIMEOUT);
        cmd_ext_out.report(e2.clone());
        l3.count_down();
        // Send the third extension.
        l4.await_timeout(WAIT_TIMEOUT);
        cmd_ext_out.report(e3.clone());
        l5.count_down();
        // Finish the fake command.
        Ok(BlazeCommandResult::success())
    }));
    let mut stub = fixture.client();

    // Act: Start the streaming RPC.
    let mut stream = fixture.block_on(run(&mut stub, create_request(&["Foo"])));
    let mut responses = Vec::new();
    let mut next = |responses: &mut Vec<RunResponse>| {
        let r = fixture.block_on(stream.message()).unwrap().unwrap();
        responses.push(r);
    };

    // Assert: Verify extensions arrive in a streaming fashion.
    next(&mut responses); // the command id
    // Wait for the first extension and check it.
    after_first_extension_latch.await_();
    next(&mut responses);
    assert_eq!(
        responses.last().unwrap().command_extensions,
        vec![command_extension1]
    );
    before_second_extension_latch.count_down();
    // Wait for the second extension and check it.
    after_second_extension_latch.await_();
    next(&mut responses);
    assert_eq!(
        responses.last().unwrap().command_extensions,
        vec![command_extension2]
    );
    before_third_extension_latch.count_down();
    // Wait for the RPC to complete and look for the third extension in the second-to-last
    // response.
    after_third_extension_latch.await_();
    while let Ok(Some(r)) = fixture.block_on(stream.message()) {
        responses.push(r);
    }
    assert_eq!(
        responses[responses.len() - 2].command_extensions,
        vec![command_extension3]
    );

    // Clean up RPC and server.
    drop(stream);
    drop(stub);
    fixture.shutdown_and_await();
}

#[test]
fn test_closing_client_should_interrupt() {
    let done = CountDownLatch::new(1);
    let d = done.clone();
    let fixture = create_server(dispatcher(move |_, _, _, _, _| {
        let monitor = Monitor::new(());
        let mut guard = monitor.lock();
        assert!(monitor.wait(&mut guard, None).is_err());
        // The only way this can happen is if the current thread is interrupted.
        d.count_down();
        Ok(BlazeCommandResult::failure_detail(
            interrupted_failure_detail(interrupted::Code::InterruptedUnknown),
        ))
    }));

    let mut stub = fixture.client();
    let mut stream = fixture.block_on(run(&mut stub, create_request(&["Foo"])));
    // onNext: shutdownNow and count down.
    if fixture.block_on(stream.message()).ok().flatten().is_some() {
        fixture.server.shutdown_now();
        done.count_down();
    }
    fixture.server.await_termination();
    done.await_();
}

#[test]
fn test_stream() {
    let fixture = create_server(dispatcher(|_, out_err, _, _, reporter| {
        let out = out_err.get_output_stream();
        reporter.report(int32_value(23));
        for _ in 0..10 {
            out.write(&[0u8; 1024]).unwrap();
        }
        reporter.report(int32_value(42));
        Ok(BlazeCommandResult::with_response_extensions(
            BlazeCommandResult::success(),
            vec![
                pack("google.protobuf.StringValue", &"foo".to_string()),
                pack("google.protobuf.BytesValue", &b"bar".to_vec()),
            ],
        ))
    }));

    let mut stub = fixture.client();
    let responses =
        fixture.block_on(async { collect(run(&mut stub, create_request(&["Foo"])).await).await });
    drop(stub);
    fixture.shutdown_and_await();

    assert_eq!(responses.len(), 14);
    assert!(!responses[0].finished);
    assert!(!responses[0].cookie.is_empty());
    assert!(!responses[1].finished);
    assert!(!responses[1].cookie.is_empty());
    assert_eq!(responses[1].command_extensions, vec![int32_value(23)]);
    for response in &responses[2..12] {
        assert!(!response.finished);
        assert_eq!(response.standard_output, vec![0u8; 1024]);
        assert!(response.command_extensions.is_empty());
    }
    assert!(!responses[12].finished);
    assert!(!responses[12].cookie.is_empty());
    assert_eq!(responses[12].command_extensions, vec![int32_value(42)]);
    assert!(responses[13].finished);
    assert_eq!(responses[13].exit_code, 0);
    assert!(responses[13].failure_detail.is_none());
    assert_eq!(
        responses[13].command_extensions,
        vec![
            pack("google.protobuf.StringValue", &"foo".to_string()),
            pack("google.protobuf.BytesValue", &b"bar".to_vec()),
        ]
    );
}

#[test]
fn bad_cookie() {
    run_bad_command_test(
        RunRequest {
            cookie: "bad-cookie".to_string(),
            client_description: "client-description".to_string(),
            ..Default::default()
        },
        FailureDetail {
            message: "Invalid RunRequest: bad cookie".to_string(),
            category: Some(Category::GrpcServer(GrpcServer {
                code: grpc_server::Code::BadCookie as i32,
            })),
        },
    );
}

#[test]
fn empty_client_description() {
    run_bad_command_test(
        RunRequest {
            cookie: REQUEST_COOKIE.to_string(),
            client_description: String::new(),
            ..Default::default()
        },
        FailureDetail {
            message: "Invalid RunRequest: no client description".to_string(),
            category: Some(Category::GrpcServer(GrpcServer {
                code: grpc_server::Code::NoClientDescription as i32,
            })),
        },
    );
}

fn run_bad_command_test(mut run_request: RunRequest, failure_detail: FailureDetail) {
    let fixture = create_server(throwing_dispatcher());
    let mut stub = fixture.client();

    run_request.arg.push(b"Foo".to_vec());
    let responses = fixture.block_on(async { collect(run(&mut stub, run_request).await).await });
    drop(stub);
    fixture.shutdown_and_await();

    assert_eq!(responses.len(), 1);
    assert!(responses[0].finished);
    assert_eq!(responses[0].exit_code, 36);
    assert_eq!(responses[0].failure_detail, Some(failure_detail));
}

#[test]
#[ignore = "needs InvocationPolicyParser (not ported, decision D14): parse_policy calls unported!, \
            which on a command thread halts the process"]
fn unparseable_invocation_policy() {
    let fixture = create_server(throwing_dispatcher());
    let mut stub = fixture.client();

    let request = RunRequest {
        invocation_policy: "invalid-invocation-policy".to_string(),
        ..create_request(&["Foo"])
    };
    let responses = fixture.block_on(async { collect(run(&mut stub, request).await).await });
    drop(stub);
    fixture.shutdown_and_await();

    assert_eq!(responses.len(), 3);
    assert!(responses[2].finished);
    assert_eq!(responses[2].exit_code, 2);
    assert_eq!(
        responses[2].failure_detail,
        Some(FailureDetail {
            message: "Invocation policy parsing failed: Malformed value of --invocation_policy: \
                      invalid-invocation-policy"
                .to_string(),
            category: Some(Category::Command(blaze_protos::failure_details::Command {
                code: command::Code::InvocationPolicyParseFailure as i32,
            })),
        })
    );
}

#[test]
fn test_interrupt_stream() {
    let fixture = create_server(dispatcher(|_, out_err, _, _, _| {
        let out = out_err.get_output_stream();
        loop {
            if thread_interrupt::interrupted() {
                return Ok(BlazeCommandResult::failure_detail(
                    interrupted_failure_detail(interrupted::Code::InterruptedUnknown),
                ));
            }
            out.write(&[0u8; 1024]).unwrap();
        }
    }));

    let mut stub = fixture.client();
    let mut stream = fixture.block_on(run(&mut stub, create_request(&["Foo"])));
    let mut responses = 0;
    // onError or onCompleted ends the loop.
    while let Ok(Some(_)) = fixture.block_on(stream.message()) {
        responses += 1;
        if responses == 10 {
            fixture.server.shutdown_now();
        }
    }
    fixture.server.await_termination();
    assert!(responses >= 10);
}

#[test]
fn test_cancel() {
    let fixture = create_server(dispatcher(|_, _, _, _, _| {
        let monitor = Monitor::new(());
        let mut guard = monitor.lock();
        monitor.wait(&mut guard, None)?;
        // Interruption expected before this is reached.
        panic!("IllegalStateException");
    }));

    let mut stub = fixture.client();
    let mut stream = fixture.block_on(run(&mut stub, create_request(&["Foo"])));
    // Wait until we've got the command id.
    let command_id = fixture
        .block_on(stream.message())
        .unwrap()
        .unwrap()
        .command_id;

    let cancel_request = CancelRequest {
        cookie: REQUEST_COOKIE.to_string(),
        command_id,
    };
    let cancel_response: CancelResponse = fixture
        .block_on(stub.cancel(cancel_request))
        .unwrap()
        .into_inner();
    let _ = cancel_response;
    let second_response = fixture.block_on(stream.message()).unwrap().unwrap();
    drop(stream);
    drop(stub);
    fixture.shutdown_and_await();

    assert!(second_response.finished);
    assert_eq!(second_response.exit_code, 8);
    let failure_detail = second_response.failure_detail.unwrap();
    match failure_detail.category {
        Some(Category::Interrupted(i)) => {
            assert_eq!(i.code, interrupted::Code::Interrupted as i32)
        }
        other => panic!("not an Interrupted failure detail: {other:?}"),
    }
}

/// A dispatcher that implements only the terminal-size overload.
struct TerminalSizeDispatcher {
    exec_started: Arc<CountDownLatch>,
    allow_listener_registration: Arc<CountDownLatch>,
    resize_seen: Arc<CountDownLatch>,
    columns_seen: Arc<AtomicI32>,
    rows_seen: Arc<AtomicI32>,
}

impl CommandDispatcher for TerminalSizeDispatcher {
    fn exec(
        &self,
        _: InvocationPolicy,
        _: Vec<String>,
        _: OutErr,
        _: LockingMode,
        _: UiVerbosity,
        _: String,
        _: i64,
        _: Option<Vec<(String, String)>>,
        _: IdleTaskResultsSupplier,
        _: Vec<Any>,
        _: Arc<dyn CommandExtensionReporter>,
    ) -> Result<BlazeCommandResult, InterruptedException> {
        panic!("AssertionError: exec overload without terminal size monitor called");
    }

    fn exec_with_terminal_size_monitor(
        &self,
        _: InvocationPolicy,
        _: Vec<String>,
        _: OutErr,
        _: LockingMode,
        _: UiVerbosity,
        _: String,
        _: i64,
        _: Option<Vec<(String, String)>>,
        _: IdleTaskResultsSupplier,
        _: Vec<Any>,
        _: Arc<dyn CommandExtensionReporter>,
        terminal_size_monitor: Arc<TerminalSizeMonitor>,
    ) -> Result<BlazeCommandResult, InterruptedException> {
        self.exec_started.count_down();
        assert!(self.allow_listener_registration.await_timeout(WAIT_TIMEOUT));
        let (c, r, seen) = (
            self.columns_seen.clone(),
            self.rows_seen.clone(),
            self.resize_seen.clone(),
        );
        terminal_size_monitor.add_listener(Arc::new(move |columns: i32, rows: i32| {
            c.store(columns, Ordering::SeqCst);
            r.store(rows, Ordering::SeqCst);
            seen.count_down();
        }));
        assert!(self.resize_seen.await_timeout(WAIT_TIMEOUT));
        Ok(BlazeCommandResult::success())
    }
}

#[test]
fn update_terminal_size_updates_running_command_monitor() {
    let exec_started = CountDownLatch::new(1);
    let allow_listener_registration = CountDownLatch::new(1);
    let resize_seen = CountDownLatch::new(1);
    let columns_seen = Arc::new(AtomicI32::new(0));
    let rows_seen = Arc::new(AtomicI32::new(0));

    let fixture = create_server(Arc::new(TerminalSizeDispatcher {
        exec_started: exec_started.clone(),
        allow_listener_registration: allow_listener_registration.clone(),
        resize_seen: resize_seen.clone(),
        columns_seen: columns_seen.clone(),
        rows_seen: rows_seen.clone(),
    }));
    let mut stub = fixture.client();

    let mut stream = fixture.block_on(run(&mut stub, create_request(&["Foo"])));
    let command_id = fixture
        .block_on(stream.message())
        .unwrap()
        .unwrap()
        .command_id;
    assert!(exec_started.await_timeout(WAIT_TIMEOUT));

    let terminal_size_request = TerminalSizeRequest {
        cookie: REQUEST_COOKIE.to_string(),
        command_id,
        columns: 47,
        rows: 12,
    };
    let terminal_size_response: TerminalSizeResponse = fixture
        .block_on(stub.update_terminal_size(terminal_size_request))
        .unwrap()
        .into_inner();

    allow_listener_registration.count_down();
    let rest = fixture.block_on(collect(stream));
    assert!(rest.last().unwrap().finished);
    drop(stub);
    fixture.shutdown_and_await();

    assert_eq!(terminal_size_response.cookie, "response-cookie");
    assert_eq!(columns_seen.load(Ordering::SeqCst), 47);
    assert_eq!(rows_seen.load(Ordering::SeqCst), 12);
}

/// The dispatcher of the preemption tests: "Foo" sleeps until interrupted.
fn preempt_dispatcher(first_command_arg: &'static str) -> Arc<dyn CommandDispatcher> {
    dispatcher(move |args, _, _, _, _| {
        if args.iter().any(|a| a == first_command_arg) {
            loop {
                if interruptible::sleep(WAIT_TIMEOUT).is_err() {
                    return Ok(BlazeCommandResult::failure_detail(
                        interrupted_failure_detail(interrupted::Code::Interrupted),
                    ));
                }
            }
        } else {
            Ok(BlazeCommandResult::success())
        }
    })
}

fn run_preempt_test(second_preemptible: bool) {
    let first_command_arg = "Foo";
    let second_command_arg = "Bar";
    let fixture = create_server(preempt_dispatcher(first_command_arg));

    let mut stub = fixture.client();
    let mut foo_stream = fixture.block_on(run(
        &mut stub,
        create_preemptible_request(&[first_command_arg]),
    ));
    // Wait for the first command to startup
    let first_foo = fixture.block_on(foo_stream.message()).unwrap().unwrap();
    let foo_rest = fixture.spawn(collect(foo_stream));

    let bar_request = if second_preemptible {
        create_preemptible_request(&[second_command_arg])
    } else {
        create_request(&[second_command_arg])
    };
    let bar = fixture.block_on(run(&mut stub, bar_request));
    let bar_responses = fixture.block_on(collect(bar));
    let foo_responses = fixture.block_on(foo_rest).unwrap();
    drop(stub);
    fixture.shutdown_and_await();

    let last_bar_response = bar_responses.last().unwrap();
    let last_foo_response = foo_responses.last().unwrap_or(&first_foo);
    assert!(last_bar_response.finished);
    assert_eq!(last_bar_response.exit_code, 0);
    assert!(last_foo_response.finished);
    assert_eq!(last_foo_response.exit_code, 8);
    match &last_foo_response.failure_detail.as_ref().unwrap().category {
        Some(Category::Interrupted(i)) => {
            assert_eq!(i.code, interrupted::Code::Interrupted as i32)
        }
        other => panic!("not an Interrupted failure detail: {other:?}"),
    }
}

/// Ensure that if a command is marked as preemptible, running a second command interrupts the
/// first command.
#[test]
fn test_preeempt() {
    run_preempt_test(false);
}

/// Ensure that if a command is marked as preemptible, running a second preemptible command
/// interrupts the first command.
#[test]
fn test_multi_preeempt() {
    run_preempt_test(true);
}

/// Ensure that when a command is not marked as preemptible, running a second command does not
/// interrupt the first command.
#[test]
fn test_no_preeempt() {
    let first_command_arg = "Foo";
    let second_command_arg = "Bar";

    let foo_blocked = CountDownLatch::new(1);
    let foo_proceed = CountDownLatch::new(1);
    let bar_blocked = CountDownLatch::new(1);
    let bar_proceed = CountDownLatch::new(1);
    let (fb, fp, bb, bp) = (
        foo_blocked.clone(),
        foo_proceed.clone(),
        bar_blocked.clone(),
        bar_proceed.clone(),
    );
    let fixture = create_server(dispatcher(move |args, _, _, _, _| {
        if args.iter().any(|a| a == first_command_arg) {
            fb.count_down();
            fp.await_();
        } else {
            bb.count_down();
            bp.await_();
        }
        Ok(BlazeCommandResult::success())
    }));

    let mut stub = fixture.client();
    let foo_stream = fixture.block_on(run(&mut stub, create_request(&[first_command_arg])));
    let foo_responses = fixture.spawn(collect(foo_stream));
    foo_blocked.await_();

    let bar = fixture.block_on(run(&mut stub, create_request(&[second_command_arg])));
    let bar_responses = fixture.spawn(collect(bar));
    bar_blocked.await_();

    // At this point both commands should be blocked on proceed latch, carry on...
    foo_proceed.count_down();
    bar_proceed.count_down();

    let foo_responses = fixture.block_on(foo_responses).unwrap();
    let bar_responses = fixture.block_on(bar_responses).unwrap();
    drop(stub);
    fixture.shutdown_and_await();

    let last_foo_response = foo_responses.last().unwrap();
    let last_bar_response = bar_responses.last().unwrap();
    assert!(last_foo_response.finished);
    assert_eq!(last_foo_response.exit_code, 0);
    assert!(last_bar_response.finished);
    assert_eq!(last_bar_response.exit_code, 0);
}

/// A `CommandServerImplBase` whose `run` hands a `BlockingStreamObserver` to a thread.
#[derive(Clone)]
struct ObserverServer {
    body: Arc<dyn Fn(Arc<BlockingStreamObserver<RunResponse>>) + Send + Sync>,
}

#[tonic::async_trait]
impl CommandServer for ObserverServer {
    type RunStream = CommandServerRunStream;

    async fn run(
        &self,
        _request: Request<RunRequest>,
    ) -> Result<Response<Self::RunStream>, Status> {
        let (observer, stream) = BlockingStreamObserver::new();
        let body = self.body.clone();
        std::thread::spawn(move || body(observer));
        Ok(Response::new(stream))
    }

    async fn cancel(
        &self,
        _request: Request<CancelRequest>,
    ) -> Result<Response<CancelResponse>, Status> {
        Err(Status::unimplemented("cancel"))
    }

    async fn update_terminal_size(
        &self,
        _request: Request<TerminalSizeRequest>,
    ) -> Result<Response<TerminalSizeResponse>, Status> {
        Err(Status::unimplemented("updateTerminalSize"))
    }

    async fn ping(&self, _request: Request<PingRequest>) -> Result<Response<PingResponse>, Status> {
        Err(Status::unimplemented("ping"))
    }
}

fn kilobyte_response() -> RunResponse {
    RunResponse {
        standard_output: vec![0u8; 1024],
        ..Default::default()
    }
}

#[test]
fn test_flow_control() {
    // This test attempts to verify that FlowControl successfully blocks after some number of
    // onNext calls (however long it takes to fill up gRPCs internal buffers). In order to
    // trigger this behavior, we intentionally block the client after a few successful calls,
    // then wait a bit, and then check that the server has stopped prematurely. Unfortunately, we
    // cannot deterministically verify that the onNext call is blocking. A faulty implementation
    // of FlowControl could pass this test if the sleep is too short. However, a correct
    // implementation should never fail this test.
    let server_done = CountDownLatch::new(1);
    let sent_count = Arc::new(AtomicUsize::new(0));
    let (sd, sc) = (server_done.clone(), sent_count.clone());
    let server_impl = ObserverServer {
        body: Arc::new(move |observer| {
            let response = kilobyte_response();
            for _ in 0..100 {
                observer.on_next(response.clone());
                sc.fetch_add(1, Ordering::SeqCst);
            }
            observer.on_completed();
            sd.count_down();
        }),
    };
    let fixture = start_server(server_impl, tempfile::tempdir().unwrap());

    let mut stub = fixture.client_with_window(Some(FLOW_CONTROL_WINDOW));
    let mut stream = fixture.block_on(run(&mut stub, RunRequest::default()));
    let mut receive_count = 0;
    // The client blocks (stops reading) once the server has sent 3 messages.
    while sent_count.load(Ordering::SeqCst) < 3 {
        fixture.block_on(stream.message()).unwrap().unwrap();
        receive_count += 1;
    }
    // Wait a bit for the server to (hopefully) block. If the server does not block, then this
    // may be flaky.
    std::thread::sleep(Duration::from_millis(10));
    let sent_while_blocked = sent_count.load(Ordering::SeqCst);
    // clientUnblocks
    while let Ok(Some(_)) = fixture.block_on(stream.message()) {
        receive_count += 1;
    }
    server_done.await_();
    drop(stream);
    drop(stub);
    fixture.shutdown_and_await();
    assert!(
        sent_while_blocked < 5,
        "sent {sent_while_blocked} while the client was blocked"
    );
    assert_eq!(receive_count, 100);
}

#[test]
fn test_flow_control_client_cancel() {
    // This test attempts to verify that FlowControl unblocks if the client prematurely closes
    // the connection. In that case, FlowControl should observe the onCancel event and interrupt
    // the calling thread.
    let server_done = CountDownLatch::new(1);
    let interrupted_at_end = Arc::new(Mutex::new(None));
    let (sd, ie) = (server_done.clone(), interrupted_at_end.clone());
    let server_impl = ObserverServer {
        body: Arc::new(move |observer| {
            let response = kilobyte_response();
            for _ in 0..100 {
                observer.on_next(response.clone());
            }
            // FlowControl should have interrupted the current thread after learning of the
            // server cancel.
            *ie.lock() = Some(thread_interrupt::is_current_interrupted());
            observer.on_completed();
            sd.count_down();
        }),
    };
    let fixture = start_server(server_impl, tempfile::tempdir().unwrap());

    let mut stub = fixture.client_with_window(Some(FLOW_CONTROL_WINDOW));
    let mut stream = fixture.block_on(run(&mut stub, RunRequest::default()));
    let mut receive_count = 0;
    loop {
        if receive_count > 3 {
            // channel.shutdownNow()
            break;
        }
        match fixture.block_on(stream.message()) {
            Ok(Some(_)) => receive_count += 1,
            _ => break,
        }
    }
    drop(stream);
    drop(stub);
    server_done.await_();
    fixture.shutdown_and_await();
    assert_eq!(*interrupted_at_end.lock(), Some(true));
}

#[test]
fn test_interrupt_flow_control() {
    // This test attempts to verify that FlowControl does not hang if the current thread is
    // interrupted. The initial implementation of FlowControl (which was never submitted) would
    // go into an infinite loop holding the lock on FlowControl. This would prevent any other
    // thread from obtaining the lock on FlowControl, and hang the entire process.
    let server_done = CountDownLatch::new(1);
    let sent_count = Arc::new(AtomicUsize::new(0));
    let (sd, sc) = (server_done.clone(), sent_count.clone());
    let server_impl = ObserverServer {
        body: Arc::new(move |observer| {
            let response = kilobyte_response();
            // We want to trigger isReady() -> false, and we use sentCount to control whether to
            // sleep on the client side. Therefore, we only set sentCount after isReady()
            // changes.
            let mut sent = 0;
            while observer.is_ready() {
                observer.on_next(response.clone());
                sent += 1;
            }
            sc.store(sent, Ordering::SeqCst);
            // If the current thread is interrupted, the subsequent onNext calls should not hang,
            // but complete eventually (they may block on flow control).
            thread_interrupt::interrupt_current();
            for _ in 0..10 {
                observer.on_next(response.clone());
                sc.fetch_add(1, Ordering::SeqCst);
            }
            observer.on_completed();
            sd.count_down();
        }),
    };
    let fixture = start_server(server_impl, tempfile::tempdir().unwrap());

    let mut stub = fixture.client_with_window(Some(FLOW_CONTROL_WINDOW));
    let mut stream = fixture.block_on(run(&mut stub, RunRequest::default()));
    let mut receive_count = 0;
    while let Ok(Some(_)) = fixture.block_on(stream.message()) {
        if sent_count.load(Ordering::SeqCst) == 0 {
            std::thread::sleep(Duration::from_millis(1));
        }
        receive_count += 1;
    }
    server_done.await_();
    assert_eq!(sent_count.load(Ordering::SeqCst), receive_count);
    drop(stream);
    drop(stub);
    fixture.shutdown_and_await();
}

struct LatchTask(Arc<CountDownLatch>);

impl IdleTask for LatchTask {
    fn display_name(&self) -> String {
        "task".to_string()
    }
    fn run(&self) -> Result<(), IdleTaskError> {
        self.0.count_down();
        Ok(())
    }
}

#[test]
fn test_idle_tasks() {
    let idle_task_running = CountDownLatch::new(1);
    let idle_task_results: Arc<Mutex<Option<Option<Vec<IdleTaskResult>>>>> =
        Arc::new(Mutex::new(None));

    let idle_task: Arc<dyn IdleTask> = Arc::new(LatchTask(idle_task_running.clone()));
    let results = idle_task_results.clone();
    let fixture = create_server(dispatcher(move |args, _, supplier, _, _| {
        if args.iter().any(|a| a == "1") {
            Ok(BlazeCommandResult::with_idle_tasks(
                BlazeCommandResult::success(),
                vec![idle_task.clone()],
            ))
        } else if args.iter().any(|a| a == "2") {
            *results.lock() = Some(supplier());
            Ok(BlazeCommandResult::success())
        } else {
            panic!("IllegalStateException: Unexpected command");
        }
    }));
    let mut stub = fixture.client();

    fixture.block_on(async { collect(run(&mut stub, create_request(&["1"])).await).await });

    idle_task_running.await_();

    fixture.block_on(async { collect(run(&mut stub, create_request(&["2"])).await).await });

    drop(stub);
    fixture.shutdown_and_await();

    let results: Vec<IdleTaskResult> = idle_task_results
        .lock()
        .clone()
        .unwrap()
        .unwrap()
        .into_iter()
        .map(|s| IdleTaskResult::new(s.name(), s.status(), Duration::ZERO))
        .collect();
    assert_eq!(
        results,
        vec![IdleTaskResult::new(
            "task",
            IdleTaskStatus::SUCCESS,
            Duration::ZERO
        )]
    );
}
