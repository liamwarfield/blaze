//! Port of
//! `src/test/java/com/google/devtools/build/lib/starlarkdebug/server/StarlarkDebugIntegrationTest.java`:
//! builds with `--experimental_skylark_debug` through the in-process runtime, with a debug
//! client (Java's `MockDebugClient`, reduced to requests and their responses) connecting to
//! the server's ephemeral port.

use std::collections::HashMap;
use std::io::Write;
use std::net::TcpStream;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use blaze_buildtool_testing::BuildIntegrationTestCase;
use blaze_events::Event;
use blaze_protos::starlark_debugging::{
    Breakpoint, DebugEvent, DebugRequest, Location, SetBreakpointsRequest, StartDebuggingRequest,
    breakpoint, debug_request,
};
use blaze_starlarkdebug::transport::{read_delimited, set_on_listen_port_callback_for_tests};
use prost::Message;

// Java: src/test/java/com/google/devtools/build/lib/starlarkdebug/server/StarlarkDebugIntegrationTest.java:62 StarlarkDebugIntegrationTest.sequenceIds
static SEQUENCE_IDS: AtomicI64 = AtomicI64::new(1);

/// Java `MockDebugClient`: requests and their responses.
// Java: src/test/java/com/google/devtools/build/lib/starlarkdebug/server/MockDebugClient.java:36 MockDebugClient
struct MockDebugClient {
    socket: TcpStream,
    responses: Arc<(Mutex<HashMap<i64, DebugEvent>>, Condvar)>,
}

impl MockDebugClient {
    // Java: src/test/java/com/google/devtools/build/lib/starlarkdebug/server/MockDebugClient.java:49 MockDebugClient.connect
    fn connect(port: u16, timeout: Duration) -> MockDebugClient {
        let start = Instant::now();
        loop {
            match TcpStream::connect(("127.0.0.1", port)) {
                Ok(socket) => {
                    let responses: Arc<(Mutex<HashMap<i64, DebugEvent>>, Condvar)> = Arc::default();
                    let mut reader = socket.try_clone().unwrap();
                    let sink = responses.clone();
                    std::thread::spawn(move || {
                        while let Ok(Some(event)) = read_delimited::<DebugEvent>(&mut reader) {
                            if event.sequence_number != 0 {
                                let (lock, cvar) = &*sink;
                                lock.lock().unwrap().insert(event.sequence_number, event);
                                cvar.notify_all();
                            }
                        }
                    });
                    return MockDebugClient { socket, responses };
                }
                Err(e) if start.elapsed() > timeout => {
                    panic!("Couldn't connect to the debug server: {e}")
                }
                Err(_) => std::thread::sleep(Duration::from_millis(100)),
            }
        }
    }

    // Java: src/test/java/com/google/devtools/build/lib/starlarkdebug/server/MockDebugClient.java:111 MockDebugClient.sendRequestAndWaitForResponse
    fn send_request_and_wait_for_response(&self, request: DebugRequest) -> Option<DebugEvent> {
        let mut socket = &self.socket;
        socket
            .write_all(&request.encode_length_delimited_to_vec())
            .unwrap();
        socket.flush().unwrap();
        let start = Instant::now();
        let (lock, cvar) = &*self.responses;
        let mut responses = lock.lock().unwrap();
        loop {
            if let Some(r) = responses.remove(&request.sequence_number) {
                return Some(r);
            }
            if start.elapsed() > Duration::from_secs(10) {
                return None;
            }
            responses = cvar
                .wait_timeout(responses, Duration::from_secs(1))
                .unwrap()
                .0;
        }
    }
}

struct Fixture {
    t: BuildIntegrationTestCase,
    /// Java's `eventCollector` (`EventKind.ALL_EVENTS`).
    events: Arc<Mutex<Vec<Event>>>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        set_on_listen_port_callback_for_tests(None);
    }
}

impl Fixture {
    // Java: src/test/java/com/google/devtools/build/lib/starlarkdebug/server/StarlarkDebugIntegrationTest.java:73 StarlarkDebugIntegrationTest.setup
    // (StarlarkDebuggerModule is one of Bazel's modules, so the runtime has it already.)
    fn set_up() -> Fixture {
        let mut t = BuildIntegrationTestCase::new();
        t.add_options(&[
            "--experimental_skylark_debug",
            "--experimental_skylark_debug_server_port=0",
        ]);
        let events: Arc<Mutex<Vec<Event>>> = Arc::default();
        let collected = events.clone();
        t.on_before_command(move |env| {
            env.get_reporter()
                .add_handler(Arc::new(AllEventsCollector(collected.clone())));
        });
        Fixture { t, events }
    }

    /// Builds `target` with `client_setup` run (on a thread of its own) when the debug server
    /// listens. `Err` when the build fails.
    // Java: src/test/java/com/google/devtools/build/lib/starlarkdebug/server/StarlarkDebugIntegrationTest.java:163 StarlarkDebugIntegrationTest.buildTarget
    fn build_target(
        &mut self,
        client_setup: impl Fn(u16) + Send + Sync + Clone + 'static,
        target: &str,
    ) -> Result<(), ()> {
        set_on_listen_port_callback_for_tests(Some(Box::new(move |port| {
            let setup = client_setup.clone();
            std::thread::spawn(move || setup(port));
        })));
        if self.t.build_target(&[target]).is_success() {
            Ok(())
        } else {
            Err(())
        }
    }

    fn messages(&self) -> Vec<String> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .map(|e| e.get_message().to_string())
            .collect()
    }
}

struct AllEventsCollector(Arc<Mutex<Vec<Event>>>);

impl blaze_events::EventHandler for AllEventsCollector {
    fn handle(&self, event: &Event) {
        self.0.lock().unwrap().push(event.clone());
    }
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdebug/server/StarlarkDebugIntegrationTest.java:171 StarlarkDebugIntegrationTest.createClient
fn create_client(debug_port: u16) -> MockDebugClient {
    MockDebugClient::connect(debug_port, Duration::from_secs(5))
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdebug/server/StarlarkDebugIntegrationTest.java:177 StarlarkDebugIntegrationTest.startDebugging
fn start_debugging(client: &MockDebugClient) {
    client.send_request_and_wait_for_response(DebugRequest {
        sequence_number: SEQUENCE_IDS.fetch_add(1, Ordering::SeqCst),
        payload: Some(debug_request::Payload::StartDebugging(
            StartDebuggingRequest {},
        )),
    });
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdebug/server/StarlarkDebugIntegrationTest.java:189 StarlarkDebugIntegrationTest.createClientAndStartDebugging
fn create_client_and_start_debugging(debug_port: u16) {
    let client = create_client(debug_port);
    start_debugging(&client);
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdebug/server/StarlarkDebugIntegrationTest.java:194 StarlarkDebugIntegrationTest.createClientAndSetBreakpoints
fn create_client_and_set_breakpoints(debug_port: u16, paths: &[String]) {
    let client = create_client(debug_port);
    set_breakpoints(&client, paths);
    start_debugging(&client);
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdebug/server/StarlarkDebugIntegrationTest.java:200 StarlarkDebugIntegrationTest.setBreakpoints
fn set_breakpoints(client: &MockDebugClient, paths: &[String]) {
    let breakpoints = paths
        .iter()
        .map(|path| Breakpoint {
            condition: Some(breakpoint::Condition::Location(Location {
                path: path.clone(),
                line_number: 0,
                column_number: 0,
            })),
            expression: String::new(),
        })
        .collect();
    client.send_request_and_wait_for_response(DebugRequest {
        sequence_number: SEQUENCE_IDS.fetch_add(1, Ordering::SeqCst),
        payload: Some(debug_request::Payload::SetBreakpoints(
            SetBreakpointsRequest {
                breakpoint: breakpoints,
            },
        )),
    });
}

const FOO_BUILD: &str = "genrule(name = 'foo', outs = ['foo.out'], cmd = 'touch $@')";

// Java: src/test/java/com/google/devtools/build/lib/starlarkdebug/server/StarlarkDebugIntegrationTest.java:80 StarlarkDebugIntegrationTest.testAnalysisResetBlocksOnDebuggingStart
#[test]
fn test_analysis_reset_blocks_on_debugging_start() {
    let mut f = Fixture::set_up();
    f.t.add_options(&["--experimental_skylark_debug_reset_analysis"]);
    f.t.write("foo/BUILD", &[FOO_BUILD]);

    // run async, otherwise this will just block on the result indefinitely
    let (tx, rx) = mpsc::channel();
    let clients: Arc<Mutex<Vec<MockDebugClient>>> = Arc::default();
    let kept = clients.clone();
    let builder = std::thread::spawn(move || {
        let r = f.build_target(
            move |port| kept.lock().unwrap().push(create_client(port)),
            "//foo",
        );
        let _ = tx.send(r);
        f
    });

    assert!(
        rx.recv_timeout(Duration::from_secs(10)).is_err(),
        "the build finished without a StartDebugging request"
    );
    // Java leaves the build blocked; here the client closes its connection so that the build
    // ends (the server's listener thread closes the server, which releases the latch) and the
    // fixture can be dropped.
    for client in clients.lock().unwrap().iter() {
        let _ = client.socket.shutdown(std::net::Shutdown::Both);
    }
    let _ = rx.recv_timeout(Duration::from_secs(120));
    let _f = builder.join();
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdebug/server/StarlarkDebugIntegrationTest.java:101 StarlarkDebugIntegrationTest.testAnalysisResetWithNoBreakpoints
#[test]
fn test_analysis_reset_with_no_breakpoints() {
    let mut f = Fixture::set_up();
    f.t.add_options(&["--experimental_skylark_debug_reset_analysis"]);
    f.t.write("foo/BUILD", &[FOO_BUILD]);

    let result = f.build_target(|port| create_client_and_set_breakpoints(port, &[]), "//foo");

    assert!(result.is_ok());
    let messages = f.messages();
    assert!(
        !messages
            .iter()
            .any(|m| m.contains("did not receive breakpoints")),
        "{messages:?}"
    );
    assert!(
        messages
            .iter()
            .any(|m| m.contains("resetting analysis for: []")),
        "{messages:?}"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdebug/server/StarlarkDebugIntegrationTest.java:114 StarlarkDebugIntegrationTest.testAnalysisResetWithBreakpoint
#[test]
fn test_analysis_reset_with_breakpoint() {
    let mut f = Fixture::set_up();
    f.t.add_options(&["--experimental_skylark_debug_reset_analysis"]);
    f.t.write("foo/BUILD", &[FOO_BUILD]);

    let foo_build = f.t.get_workspace().join("foo/BUILD").display().to_string();
    let result = f.build_target(
        move |port| create_client_and_set_breakpoints(port, std::slice::from_ref(&foo_build)),
        "//foo",
    );

    let messages = f.messages();
    assert!(
        messages
            .iter()
            .any(|m| m.starts_with("resetting analysis for: ") && m.contains("/foo/BUILD")),
        "{messages:?}"
    );
    assert!(result.is_ok());
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdebug/server/StarlarkDebugIntegrationTest.java:128 StarlarkDebugIntegrationTest.testAnalysisResetWithBreakpointDeletesSkyframeFileNode
#[test]
#[ignore = "the graph listener of blaze_skyframe has no INVALIDATION reason (QueryableGraph.Reason \
            has only PRE_OR_POST_EVALUATION) and a delete emits no listener event, so the deleted \
            FILE nodes cannot be observed"]
fn test_analysis_reset_with_breakpoint_deletes_skyframe_file_node() {
    let mut f = Fixture::set_up();
    f.t.write("foo/BUILD", &[FOO_BUILD]);
    // first build to populate skyframe
    assert!(
        f.build_target(create_client_and_start_debugging, "//foo")
            .is_ok()
    );
    blaze_util::unported!("graph listener: FILE keys deleted with Reason.INVALIDATION");
}
