//! Port of `StarlarkDebugServerTest` (and its `MockDebugClient`): the debug server over a real
//! socket, with Starlark files executed in worker threads.
//!
//! Java: src/test/java/com/google/devtools/build/lib/starlarkdebug/server/StarlarkDebugServerTest.java
//! Java: src/test/java/com/google/devtools/build/lib/starlarkdebug/server/MockDebugClient.java
//!
//! The debugger is process-global (Java `Debug.setDebugger`), so the tests take a lock and run
//! one at a time.

use std::collections::HashMap;
use std::io::Write;
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, LazyLock, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use blaze_events::StoredEventHandler;
use blaze_protos::starlark_debugging::{
    Breakpoint, ContinueExecutionRequest, DebugEvent, DebugRequest, Error as ErrorProto,
    EvaluateRequest, Frame, GetChildrenRequest, ListFramesRequest, ListFramesResponse, Location,
    PauseReason, PausedThread, Scope, SetBreakpointsRequest, StartDebuggingRequest,
    StartDebuggingResponse, Stepping, Value as ValueProto, breakpoint, debug_event,
    debug_request,
};
use blaze_starlark::debug::{self, TOP_LEVEL};
use blaze_starlark::eval;
use blaze_starlark::loader::LoadMap;
use blaze_starlark::thread::CollectingEvents;
use blaze_starlark::{EvalError, FileKind, Phase, StarlarkSemantics, ThreadContext};
use blaze_starlarkdebug::serialization::{DebugObject, ThreadObjectMap, get_value_proto};
use blaze_starlarkdebug::thread_handler::current_thread_id;
use blaze_starlarkdebug::transport::read_delimited;
use blaze_starlarkdebug::{NoopCallback, StarlarkDebugServer, event_helper};
use prost::Message;
use starlark::environment::{FrozenModule, Module};
use starlark::values::list::AllocList;
use starlark::values::{Heap, Value};

static SERIAL: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

const RESPONSE_TIMEOUT: Duration = Duration::from_millis(10000);

// ---------------------------------------------------------------------------------------------
// MockDebugClient

/// A basic implementation of a Starlark debugging client, for use in integration tests.
// Java: src/test/java/com/google/devtools/build/lib/starlarkdebug/server/MockDebugClient.java:36 MockDebugClient
struct MockDebugClient {
    socket: TcpStream,
    state: Arc<(Mutex<ClientState>, Condvar)>,
    read_task: Option<JoinHandle<()>>,
}

#[derive(Default)]
struct ClientState {
    unnumbered_events: Vec<DebugEvent>,
    responses: HashMap<i64, DebugEvent>,
    read_task_done: bool,
}

impl MockDebugClient {
    /// Connects to the debug server, and starts listening for events.
    fn connect(port: u16, timeout: Duration) -> MockDebugClient {
        let start = Instant::now();
        let mut last_error = None;
        while start.elapsed() < timeout {
            match TcpStream::connect(("127.0.0.1", port)) {
                Ok(socket) => {
                    let state: Arc<(Mutex<ClientState>, Condvar)> = Arc::default();
                    let mut reader = socket.try_clone().unwrap();
                    let task_state = state.clone();
                    let read_task = std::thread::spawn(move || {
                        loop {
                            match read_delimited::<DebugEvent>(&mut reader) {
                                Ok(Some(event)) => event_received(&task_state, event),
                                _ => break,
                            }
                        }
                        let (lock, cvar) = &*task_state;
                        lock.lock().unwrap().read_task_done = true;
                        cvar.notify_all();
                    });
                    return MockDebugClient {
                        socket,
                        state,
                        read_task: Some(read_task),
                    };
                }
                Err(e) => last_error = Some(e),
            }
        }
        panic!("Couldn't connect to the debug server: {last_error:?}");
    }

    fn close(&mut self) {
        let _ = self.socket.shutdown(Shutdown::Both);
        if let Some(task) = self.read_task.take() {
            let _ = task.join();
        }
    }

    fn clear_unnumbered_events(&self) {
        self.state.0.lock().unwrap().unnumbered_events.clear();
    }

    /// Blocks waiting for an unnumbered event (not a direct response to a request). Returns
    /// `None` if no event arrives before the timeout.
    fn wait_for_event(
        &self,
        predicate: impl Fn(&DebugEvent) -> bool,
        timeout: Duration,
    ) -> Option<DebugEvent> {
        self.wait_for_events(|list| list.iter().any(&predicate), timeout);
        self.state
            .0
            .lock()
            .unwrap()
            .unnumbered_events
            .iter()
            .find(|e| predicate(e))
            .cloned()
    }

    /// Blocks waiting for a condition on all unnumbered events to be satisfied. Returns true if
    /// the condition was satisfied before the timeout.
    fn wait_for_events(&self, predicate: impl Fn(&[DebugEvent]) -> bool, timeout: Duration) -> bool {
        let start = Instant::now();
        let (lock, cvar) = &*self.state;
        let mut state = lock.lock().unwrap();
        while !predicate(&state.unnumbered_events) && start.elapsed() < timeout {
            state = cvar.wait_timeout(state, timeout).unwrap().0;
        }
        predicate(&state.unnumbered_events)
    }

    /// Sends a `DebugRequest` to the server, and blocks waiting for a response.
    fn send_request_and_wait_for_response(&self, request: DebugRequest) -> Option<DebugEvent> {
        let bytes = request.encode_length_delimited_to_vec();
        let mut socket = &self.socket;
        socket.write_all(&bytes).unwrap();
        socket.flush().unwrap();
        self.wait_for_response(request.sequence_number)
    }

    /// Wait for a response from the debug server. Returns `None` if no response was received.
    fn wait_for_response(&self, sequence: i64) -> Option<DebugEvent> {
        let start = Instant::now();
        let (lock, cvar) = &*self.state;
        let mut state = lock.lock().unwrap();
        loop {
            if let Some(response) = state.responses.remove(&sequence) {
                return Some(response);
            }
            if state.read_task_done || start.elapsed() >= RESPONSE_TIMEOUT {
                return None;
            }
            state = cvar.wait_timeout(state, Duration::from_secs(1)).unwrap().0;
        }
    }
}

fn event_received(state: &Arc<(Mutex<ClientState>, Condvar)>, event: DebugEvent) {
    let (lock, cvar) = &**state;
    let mut state = lock.lock().unwrap();
    if event.sequence_number == 0 {
        state.unnumbered_events.push(event);
        cvar.notify_all();
        return;
    }
    let sequence = event.sequence_number;
    if state.responses.insert(sequence, event).is_some() {
        panic!(
            "There's already an event in the response queue corresponding to sequence number \
             {sequence}"
        );
    }
    cvar.notify_all();
}

// ---------------------------------------------------------------------------------------------
// The fixture

struct Fixture {
    client: MockDebugClient,
    server: Arc<StarlarkDebugServer>,
    workers: Vec<Worker>,
    /// The next identifier of the test's own `ThreadObjectMap` (Java `dummyObjectMap`).
    dummy_next_id: i64,
    _serial: std::sync::MutexGuard<'static, ()>,
}

/// A worker thread executing a Starlark file.
struct Worker {
    id: i64,
    name: String,
    handle: Option<JoinHandle<Result<FrozenModule, EvalError>>>,
}

impl Worker {
    /// Waits for the thread to finish (after the test resumed it) and returns its module.
    fn join(&mut self) -> FrozenModule {
        self.handle
            .take()
            .unwrap()
            .join()
            .expect("worker panicked")
            .expect("evaluation failed")
    }
}

impl Fixture {
    // Java: StarlarkDebugServerTest.setUpServerAndClient
    fn set_up_server_and_client() -> Fixture {
        let serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let server_socket = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = server_socket.local_addr().unwrap().port();
        let events = Arc::new(StoredEventHandler::new());
        let future = std::thread::spawn(move || {
            StarlarkDebugServer::create_and_wait_for_connection_on(
                events,
                server_socket,
                false,
                Arc::new(NoopCallback),
                None,
            )
        });
        let client = MockDebugClient::connect(port, Duration::from_secs(10));
        let server = future.join().unwrap().expect("server");
        debug::set_debugger(Some(server.clone()));
        Fixture {
            client,
            server,
            workers: Vec::new(),
            dummy_next_id: 1,
            _serial: serial,
        }
    }

    /// The `Value` proto of the given value and label, from the test's own object map: a value
    /// with children takes the map's next identifier.
    // Java: StarlarkDebugServerTest.getValueProto
    fn get_value_proto(
        &mut self,
        label: &str,
        value: impl for<'v> FnOnce(&Module<'v>) -> Value<'v>,
    ) -> ValueProto {
        let mut proto = Module::with_temp_heap(|module| {
            let v = value(&module);
            let mut map = ThreadObjectMap::default();
            get_value_proto(&mut map, label, DebugObject::Value(v))
        });
        if proto.has_children {
            proto.id = self.dummy_next_id;
            self.dummy_next_id += 1;
        }
        proto
    }

    fn set_breakpoints(&self, locations: Vec<Location>) {
        self.set_breakpoint_list(
            locations
                .into_iter()
                .map(|l| Breakpoint {
                    condition: Some(breakpoint::Condition::Location(l)),
                    expression: String::new(),
                })
                .collect(),
        );
    }

    fn set_breakpoint_list(&self, breakpoints: Vec<Breakpoint>) {
        let response = self
            .client
            .send_request_and_wait_for_response(request(
                10,
                debug_request::Payload::SetBreakpoints(SetBreakpointsRequest {
                    breakpoint: breakpoints,
                }),
            ))
            .expect("response");
        assert!(matches!(
            response.payload,
            Some(debug_event::Payload::SetBreakpoints(_))
        ));
        assert_eq!(response.sequence_number, 10);
    }

    fn send_start_debugging_request(&self) {
        self.client.send_request_and_wait_for_response(request(
            1,
            debug_request::Payload::StartDebugging(StartDebuggingRequest {}),
        ));
    }

    fn list_frames(&self, thread_id: i64) -> ListFramesResponse {
        let event = self
            .client
            .send_request_and_wait_for_response(request(
                1,
                debug_request::Payload::ListFrames(ListFramesRequest { thread_id }),
            ))
            .expect("response");
        assert_eq!(event.sequence_number, 1);
        match event.payload {
            Some(debug_event::Payload::ListFrames(frames)) => frames,
            other => panic!("expected ListFrames, got {other:?}"),
        }
    }

    fn evaluate(&self, sequence_number: i64, thread_id: i64, statement: &str) -> DebugEvent {
        self.client
            .send_request_and_wait_for_response(request(
                sequence_number,
                debug_request::Payload::Evaluate(EvaluateRequest {
                    thread_id,
                    statement: statement.to_owned(),
                }),
            ))
            .expect("response")
    }

    fn continue_execution(&self, thread_id: i64, stepping: Stepping) {
        self.client.send_request_and_wait_for_response(request(
            2,
            debug_request::Payload::ContinueExecution(ContinueExecutionRequest {
                thread_id,
                stepping: stepping as i32,
            }),
        ));
    }

    /// Creates and starts a worker thread parsing, resolving, and executing the given Starlark
    /// file in a fresh module with a default environment; returns its index in
    /// `self.workers`.
    // Java: StarlarkDebugServerTest.execInWorkerThread
    fn exec_in_worker_thread(&mut self, file_name: &str, source: &str) -> usize {
        let (tx, rx) = mpsc::channel();
        let file_name = file_name.to_owned();
        let source = source.to_owned();
        let index = self.workers.len();
        let handle = std::thread::Builder::new()
            .name(format!("Thread-{index}"))
            .stack_size(64 << 20)
            .spawn(move || {
                tx.send((
                    current_thread_id(),
                    std::thread::current().name().unwrap().to_owned(),
                ))
                .unwrap();
                let semantics = StarlarkSemantics::default();
                // Module.create(): only the universe.
                let globals = blaze_starlark::env::universe_builder(&semantics).build();
                let events = Arc::new(CollectingEvents::default());
                let ctx = ThreadContext::new(Phase::BzlLoad, Arc::new(semantics), events);
                let compiled = eval::compile(FileKind::Bzl, &file_name, source)?;
                eval::exec_module(compiled, &globals, &LoadMap::new(), &ctx, &Default::default())
            })
            .unwrap();
        let (id, name) = rx.recv().unwrap();
        self.workers.push(Worker {
            id,
            name,
            handle: Some(handle),
        });
        index
    }

    /// Resumes every thread and returns the module of worker `index` once it has finished.
    fn finish(&mut self, index: usize) -> FrozenModule {
        self.client.send_request_and_wait_for_response(request(
            99,
            debug_request::Payload::ContinueExecution(ContinueExecutionRequest {
                thread_id: 0,
                stepping: Stepping::None as i32,
            }),
        ));
        self.workers[index].join()
    }

    /// The `Value` proto of global `name` of `module`.
    fn global_proto(&mut self, module: &FrozenModule, name: &str) -> ValueProto {
        let value = module.get(name).unwrap();
        self.get_value_proto(name, |m| eval::frozen_into(m, &value))
    }
}

impl Drop for Fixture {
    // Java: StarlarkDebugServerTest.shutDown
    fn drop(&mut self) {
        self.client.close();
        debug::set_debugger(None);
        blaze_starlark::debug::Debugger::close(&*self.server);
        for worker in &mut self.workers {
            if let Some(handle) = worker.handle.take() {
                let _ = handle.join();
            }
        }
    }
}

fn request(sequence_number: i64, payload: debug_request::Payload) -> DebugRequest {
    DebugRequest {
        sequence_number,
        payload: Some(payload),
    }
}

fn location(path: &str, line: u32, column: u32) -> Location {
    Location {
        path: path.to_owned(),
        line_number: line,
        column_number: column,
    }
}

fn has_thread_paused(e: &DebugEvent) -> bool {
    matches!(e.payload, Some(debug_event::Payload::ThreadPaused(_)))
}

fn has_thread_continued(e: &DebugEvent) -> bool {
    matches!(e.payload, Some(debug_event::Payload::ThreadContinued(_)))
}

fn paused_thread(e: &DebugEvent) -> PausedThread {
    match &e.payload {
        Some(debug_event::Payload::ThreadPaused(p)) => p.thread.clone().unwrap(),
        other => panic!("expected ThreadPaused, got {other:?}"),
    }
}

fn int_list<'v>(heap: Heap<'v>, items: &[i32]) -> Value<'v> {
    heap.alloc(AllocList(items.iter().copied()))
}

/// Asserts that the given frames are equal after clearing the identifier from all values.
fn assert_frames_equal_ignoring_value_identifiers(frame1: &Frame, frame2: &Frame) {
    assert_eq!(clear_ids(frame1), clear_ids(frame2));
}

fn clear_ids(frame: &Frame) -> Frame {
    let mut frame = frame.clone();
    for scope in &mut frame.scope {
        for binding in &mut scope.binding {
            binding.id = 0;
        }
    }
    frame
}

fn assert_values_equal_ignoring_id(value1: &ValueProto, value2: &ValueProto) {
    let clear = |v: &ValueProto| ValueProto { id: 0, ..v.clone() };
    assert_eq!(clear(value1), clear(value2));
}

fn timeout() -> Duration {
    Duration::from_secs(5)
}

const BUILD: &str = "/a/build/file/BUILD";
const TWO_LISTS: &str = "x = [1,2,3]\ny = [2,3,4]\n";

// ---------------------------------------------------------------------------------------------
// Tests

#[test]
fn test_start_debugging_response_received() {
    let f = Fixture::set_up_server_and_client();
    let response = f
        .client
        .send_request_and_wait_for_response(request(
            1,
            debug_request::Payload::StartDebugging(StartDebuggingRequest {}),
        ))
        .expect("response");
    assert_eq!(
        response,
        DebugEvent {
            sequence_number: 1,
            payload: Some(debug_event::Payload::StartDebugging(
                StartDebuggingResponse {}
            )),
        }
    );
}

#[test]
fn test_paused_until_start_debugging_request_received() {
    let mut f = Fixture::set_up_server_and_client();
    let w = f.exec_in_worker_thread(BUILD, "x = [1,2,3]");
    let (thread_id, thread_name) = (f.workers[w].id, f.workers[w].name.clone());

    // wait for BUILD evaluation to start
    let event = f.client.wait_for_event(has_thread_paused, timeout()).unwrap();
    let expected_location = location(BUILD, 1, 1);
    assert_eq!(
        event,
        event_helper::thread_paused_event(PausedThread {
            id: thread_id,
            name: thread_name,
            pause_reason: PauseReason::Initializing as i32,
            location: Some(expected_location),
            conditional_breakpoint_error: None,
        })
    );

    f.send_start_debugging_request();
    let event = f
        .client
        .wait_for_event(has_thread_continued, timeout())
        .unwrap();
    assert_eq!(event, event_helper::thread_continued_event(thread_id));
}

#[test]
fn test_resume_all_threads() {
    let mut f = Fixture::set_up_server_and_client();
    f.send_start_debugging_request();
    f.set_breakpoints(vec![location(BUILD, 2, 0)]);

    // evaluate in two separate worker threads
    f.exec_in_worker_thread(BUILD, TWO_LISTS);
    f.exec_in_worker_thread(BUILD, TWO_LISTS);

    // wait for both breakpoints to be hit
    let paused = f.client.wait_for_events(
        |list| list.iter().filter(|e| has_thread_paused(e)).count() == 2,
        timeout(),
    );
    assert!(paused);

    f.client.send_request_and_wait_for_response(request(
        45,
        debug_request::Payload::ContinueExecution(ContinueExecutionRequest::default()),
    ));

    let resumed = f.client.wait_for_events(
        |list| list.iter().filter(|e| has_thread_continued(e)).count() == 2,
        timeout(),
    );
    assert!(resumed);
}

#[test]
fn test_pause_at_breakpoint() {
    let mut f = Fixture::set_up_server_and_client();
    f.send_start_debugging_request();
    let breakpoint = location(BUILD, 2, 0);
    f.set_breakpoints(vec![breakpoint.clone()]);

    let w = f.exec_in_worker_thread(BUILD, TWO_LISTS);
    let (thread_id, thread_name) = (f.workers[w].id, f.workers[w].name.clone());

    // wait for breakpoint to be hit
    let event = f.client.wait_for_event(has_thread_paused, timeout()).unwrap();
    let expected_thread_state = PausedThread {
        name: thread_name,
        id: thread_id,
        pause_reason: PauseReason::HitBreakpoint as i32,
        location: Some(Location {
            column_number: 1,
            ..breakpoint
        }),
        conditional_breakpoint_error: None,
    };
    assert_eq!(
        event,
        event_helper::thread_paused_event(expected_thread_state)
    );
}

#[test]
fn test_do_not_pause_at_unsatisfied_conditional_breakpoint() {
    let mut f = Fixture::set_up_server_and_client();
    f.send_start_debugging_request();
    let breakpoints = vec![
        Breakpoint {
            condition: Some(breakpoint::Condition::Location(location(BUILD, 2, 0))),
            expression: "x[0] == 2".to_owned(),
        },
        Breakpoint {
            condition: Some(breakpoint::Condition::Location(location(BUILD, 3, 0))),
            expression: "x[0] == 1".to_owned(),
        },
    ];
    f.set_breakpoint_list(breakpoints);

    let w = f.exec_in_worker_thread(BUILD, "x = [1,2,3]\ny = [2,3,4]\nz = 1\n");
    let (thread_id, thread_name) = (f.workers[w].id, f.workers[w].name.clone());

    let event = f.client.wait_for_event(has_thread_paused, timeout()).unwrap();
    assert_eq!(
        event,
        event_helper::thread_paused_event(PausedThread {
            name: thread_name,
            id: thread_id,
            location: Some(location(BUILD, 3, 1)),
            pause_reason: PauseReason::HitBreakpoint as i32,
            conditional_breakpoint_error: None,
        })
    );
}

#[test]
fn test_pause_at_satisfied_conditional_breakpoint() {
    let mut f = Fixture::set_up_server_and_client();
    f.send_start_debugging_request();
    f.set_breakpoint_list(vec![Breakpoint {
        condition: Some(breakpoint::Condition::Location(location(BUILD, 2, 0))),
        expression: "x[0] == 1".to_owned(),
    }]);

    let w = f.exec_in_worker_thread(BUILD, TWO_LISTS);
    let (thread_id, thread_name) = (f.workers[w].id, f.workers[w].name.clone());

    // wait for breakpoint to be hit
    let event = f.client.wait_for_event(has_thread_paused, timeout()).unwrap();
    assert_eq!(
        event,
        event_helper::thread_paused_event(PausedThread {
            name: thread_name,
            id: thread_id,
            pause_reason: PauseReason::HitBreakpoint as i32,
            location: Some(location(BUILD, 2, 1)),
            conditional_breakpoint_error: None,
        })
    );
}

#[test]
fn test_pause_at_invalid_condition_breakpoint_with_error() {
    let mut f = Fixture::set_up_server_and_client();
    f.send_start_debugging_request();
    f.set_breakpoint_list(vec![Breakpoint {
        condition: Some(breakpoint::Condition::Location(location(BUILD, 2, 0))),
        expression: "z[0] == 1".to_owned(),
    }]);

    let w = f.exec_in_worker_thread(BUILD, TWO_LISTS);
    let (thread_id, thread_name) = (f.workers[w].id, f.workers[w].name.clone());

    // wait for breakpoint to be hit
    let event = f.client.wait_for_event(has_thread_paused, timeout()).unwrap();
    assert_eq!(
        event,
        event_helper::thread_paused_event(PausedThread {
            name: thread_name,
            id: thread_id,
            pause_reason: PauseReason::ConditionalBreakpointError as i32,
            location: Some(location(BUILD, 2, 1)),
            conditional_breakpoint_error: Some(ErrorProto {
                message: "name 'z' is not defined".to_owned(),
            }),
        })
    );
}

#[test]
fn test_list_frames_for_invalid_thread() {
    let f = Fixture::set_up_server_and_client();
    f.send_start_debugging_request();
    let event = f
        .client
        .send_request_and_wait_for_response(request(
            1,
            debug_request::Payload::ListFrames(ListFramesRequest { thread_id: 20 }),
        ))
        .expect("response");
    match event.payload {
        Some(debug_event::Payload::Error(e)) => {
            assert!(e.message.contains("Thread 20 is not paused"), "{}", e.message)
        }
        other => panic!("expected an error, got {other:?}"),
    }
}

#[test]
fn test_simple_list_frames_request() {
    let mut f = Fixture::set_up_server_and_client();
    f.send_start_debugging_request();
    f.set_breakpoints(vec![location(BUILD, 2, 0)]);

    let w = f.exec_in_worker_thread(BUILD, TWO_LISTS);
    let thread_id = f.workers[w].id;

    // wait for breakpoint to be hit
    f.client.wait_for_event(has_thread_paused, timeout());

    let frames = f.list_frames(thread_id);
    assert_eq!(frames.frame.len(), 1);
    let x = f.get_value_proto("x", |m| int_list(m.heap(), &[1, 2, 3]));
    assert_frames_equal_ignoring_value_identifiers(
        &frames.frame[0],
        &Frame {
            function_name: TOP_LEVEL.to_owned(),
            location: Some(location(BUILD, 2, 1)),
            scope: vec![Scope {
                name: "global".to_owned(),
                binding: vec![x],
            }],
        },
    );
}

#[test]
fn test_list_frames_with_uninitialized_cell_request() {
    let mut f = Fixture::set_up_server_and_client();
    f.send_start_debugging_request();
    let file = "/a/build/file/foo.bzl";
    f.set_breakpoints(vec![location(file, 2, 0)]);

    let w = f.exec_in_worker_thread(
        file,
        "def outer():\n   x = [1,2,3] # <- breakpoint\n\n   def inner():\n       x.append(4)\n\n   inner()\n\nouter()\n",
    );
    let thread_id = f.workers[w].id;

    // wait for breakpoint to be hit
    f.client.wait_for_event(has_thread_paused, timeout());

    let frames = f.list_frames(thread_id);
    let module = f.finish(w);
    assert_eq!(frames.frame.len(), 2);
    let outer = f.global_proto(&module, "outer");
    // critically x is not present as a local
    assert_frames_equal_ignoring_value_identifiers(
        &frames.frame[0],
        &Frame {
            function_name: "outer".to_owned(),
            location: Some(location(file, 2, 4)),
            scope: vec![Scope {
                name: "global".to_owned(),
                binding: vec![outer.clone()],
            }],
        },
    );
    assert_frames_equal_ignoring_value_identifiers(
        &frames.frame[1],
        &Frame {
            function_name: TOP_LEVEL.to_owned(),
            location: Some(location(file, 9, 6)),
            scope: vec![Scope {
                name: "global".to_owned(),
                binding: vec![outer],
            }],
        },
    );
}

#[test]
fn test_list_frames_with_initialized_cell_request() {
    let mut f = Fixture::set_up_server_and_client();
    f.send_start_debugging_request();
    let file = "/a/build/file/foo.bzl";
    f.set_breakpoints(vec![location(file, 3, 0)]);

    let w = f.exec_in_worker_thread(
        file,
        "def outer():\n   x = [1]\n   pass          # <- breakpoint\n\n   def inner():\n       x.append(4)\n\n   inner()\n\nouter()\n",
    );
    let thread_id = f.workers[w].id;

    // wait for breakpoint to be hit
    f.client.wait_for_event(has_thread_paused, timeout());

    let frames = f.list_frames(thread_id);
    let module = f.finish(w);
    assert_eq!(frames.frame.len(), 2);
    let x = f.get_value_proto("x", |m| int_list(m.heap(), &[1]));
    let outer = f.global_proto(&module, "outer");
    // critically x is present as a local
    assert_frames_equal_ignoring_value_identifiers(
        &frames.frame[0],
        &Frame {
            function_name: "outer".to_owned(),
            location: Some(location(file, 3, 4)),
            scope: vec![
                Scope {
                    name: "local".to_owned(),
                    binding: vec![x],
                },
                Scope {
                    name: "global".to_owned(),
                    binding: vec![outer.clone()],
                },
            ],
        },
    );
    assert_frames_equal_ignoring_value_identifiers(
        &frames.frame[1],
        &Frame {
            function_name: TOP_LEVEL.to_owned(),
            location: Some(location(file, 10, 6)),
            scope: vec![Scope {
                name: "global".to_owned(),
                binding: vec![outer],
            }],
        },
    );
}

#[test]
fn test_get_children_request() {
    let mut f = Fixture::set_up_server_and_client();
    f.send_start_debugging_request();
    f.set_breakpoints(vec![location(BUILD, 2, 0)]);

    let w = f.exec_in_worker_thread(BUILD, TWO_LISTS);
    let thread_id = f.workers[w].id;

    // wait for breakpoint to be hit
    f.client.wait_for_event(has_thread_paused, timeout());

    let frames = f.list_frames(thread_id);
    let x_value = frames.frame[0].scope[0].binding[0].clone();

    let expected_x = f.get_value_proto("x", |m| int_list(m.heap(), &[1, 2, 3]));
    assert_values_equal_ignoring_id(&x_value, &expected_x);

    // Java's getChildren(xValue) reads the children of the test's own copy of the value (its
    // object map has the same identifier); they are computed the same way here.
    let children = Module::with_temp_heap(|module| {
        let list = int_list(module.heap(), &[1, 2, 3]);
        let mut map = ThreadObjectMap::default();
        blaze_starlarkdebug::serialization::get_children(
            &mut map,
            DebugObject::Value(list),
            module.heap(),
        )
    });
    let expected: Vec<ValueProto> = [(0, 1), (1, 2), (2, 3)]
        .into_iter()
        .map(|(i, n)| f.get_value_proto(&format!("[{i}]"), move |m| m.heap().alloc(n)))
        .collect();
    assert_eq!(children, expected);
}

/// Not in Java: the same children, asked of the server (`GetChildrenRequest`).
#[test]
fn get_children_request_answered_by_the_server() {
    let mut f = Fixture::set_up_server_and_client();
    f.send_start_debugging_request();
    f.set_breakpoints(vec![location(BUILD, 2, 0)]);

    let w = f.exec_in_worker_thread(BUILD, TWO_LISTS);
    let thread_id = f.workers[w].id;
    f.client.wait_for_event(has_thread_paused, timeout());

    let frames = f.list_frames(thread_id);
    let x_value = frames.frame[0].scope[0].binding[0].clone();
    let response = f
        .client
        .send_request_and_wait_for_response(request(
            7,
            debug_request::Payload::GetChildren(GetChildrenRequest {
                thread_id,
                value_id: x_value.id,
            }),
        ))
        .expect("response");
    let expected: Vec<ValueProto> = [(0, 1), (1, 2), (2, 3)]
        .into_iter()
        .map(|(i, n)| f.get_value_proto(&format!("[{i}]"), move |m| m.heap().alloc(n)))
        .collect();
    match response.payload {
        Some(debug_event::Payload::GetChildren(c)) => assert_eq!(c.children, expected),
        other => panic!("expected GetChildren, got {other:?}"),
    }
}

#[test]
fn test_list_frames_shadowed_binding() {
    let mut f = Fixture::set_up_server_and_client();
    f.send_start_debugging_request();
    let file = "/a/build/file/test.bzl";
    f.set_breakpoints(vec![location(file, 6, 0)]);

    let w = f.exec_in_worker_thread(
        file,
        "a = 1\nc = 3\ndef fn():\n  a = 2\n  b = 1\n  b + 1\nfn()\n",
    );
    let thread_id = f.workers[w].id;

    // wait for breakpoint to be hit
    f.client.wait_for_event(has_thread_paused, timeout());

    let frames = f.list_frames(thread_id);
    let module = f.finish(w);
    assert_eq!(frames.frame.len(), 2);

    let a2 = f.get_value_proto("a", |m| m.heap().alloc(2));
    let b1 = f.get_value_proto("b", |m| m.heap().alloc(1));
    let c3 = f.get_value_proto("c", |m| m.heap().alloc(3));
    let fn_ = f.global_proto(&module, "fn");
    assert_frames_equal_ignoring_value_identifiers(
        &frames.frame[0],
        &Frame {
            function_name: "fn".to_owned(),
            location: Some(location(file, 6, 3)),
            scope: vec![
                Scope {
                    name: "local".to_owned(),
                    binding: vec![a2, b1],
                },
                Scope {
                    name: "global".to_owned(),
                    binding: vec![c3.clone(), fn_.clone()],
                },
            ],
        },
    );

    let a1 = f.get_value_proto("a", |m| m.heap().alloc(1));
    assert_frames_equal_ignoring_value_identifiers(
        &frames.frame[1],
        &Frame {
            function_name: TOP_LEVEL.to_owned(),
            location: Some(location(file, 7, 3)),
            scope: vec![Scope {
                name: "global".to_owned(),
                binding: vec![a1, c3, fn_],
            }],
        },
    );
}

#[test]
fn test_evaluate_request_with_expression() {
    let mut f = Fixture::set_up_server_and_client();
    f.send_start_debugging_request();
    f.set_breakpoints(vec![location(BUILD, 2, 0)]);

    let w = f.exec_in_worker_thread(BUILD, TWO_LISTS);
    let thread_id = f.workers[w].id;

    // wait for breakpoint to be hit
    f.client.wait_for_event(has_thread_paused, timeout());

    let response = f.evaluate(123, thread_id, "x[1]");
    let expected = f.get_value_proto("Evaluation result", |m| m.heap().alloc(2));
    match response.payload {
        Some(debug_event::Payload::Evaluate(r)) => assert_eq!(r.result, Some(expected)),
        other => panic!("expected Evaluate, got {other:?}"),
    }
}

#[test]
fn test_evaluate_request_with_assignment_statement() {
    let mut f = Fixture::set_up_server_and_client();
    f.send_start_debugging_request();
    f.set_breakpoints(vec![location(BUILD, 2, 0)]);

    let w = f.exec_in_worker_thread(BUILD, TWO_LISTS);
    let thread_id = f.workers[w].id;

    // wait for breakpoint to be hit
    f.client.wait_for_event(has_thread_paused, timeout());

    let response = f.evaluate(123, thread_id, "x = [5,6]");
    let none = f.get_value_proto("Evaluation result", |_| Value::new_none());
    match response.payload {
        Some(debug_event::Payload::Evaluate(r)) => assert_eq!(r.result, Some(none)),
        other => panic!("expected Evaluate, got {other:?}"),
    }

    let frames = f.list_frames(thread_id);
    let x = f.get_value_proto("x", |m| int_list(m.heap(), &[5, 6]));
    assert!(
        frames.frame[0].scope[0].binding.contains(&x),
        "{:?}",
        frames.frame[0].scope[0].binding
    );
}

#[test]
fn test_evaluate_request_with_expression_statement_mutating_state() {
    let mut f = Fixture::set_up_server_and_client();
    f.send_start_debugging_request();
    f.set_breakpoints(vec![location(BUILD, 2, 0)]);

    let w = f.exec_in_worker_thread(BUILD, TWO_LISTS);
    let thread_id = f.workers[w].id;

    // wait for breakpoint to be hit
    f.client.wait_for_event(has_thread_paused, timeout());

    let response = f.evaluate(123, thread_id, "x.append(4)");
    let none = f.get_value_proto("Evaluation result", |_| Value::new_none());
    match response.payload {
        Some(debug_event::Payload::Evaluate(r)) => assert_eq!(r.result, Some(none)),
        other => panic!("expected Evaluate, got {other:?}"),
    }

    let frames = f.list_frames(thread_id);
    let x = f.get_value_proto("x", |m| int_list(m.heap(), &[1, 2, 3, 4]));
    assert!(
        frames.frame[0].scope[0].binding.contains(&x),
        "{:?}",
        frames.frame[0].scope[0].binding
    );
}

#[test]
fn test_evaluate_request_throwing_exception() {
    let mut f = Fixture::set_up_server_and_client();
    f.send_start_debugging_request();
    f.set_breakpoints(vec![location(BUILD, 2, 0)]);

    let w = f.exec_in_worker_thread(BUILD, TWO_LISTS);
    let thread_id = f.workers[w].id;

    // wait for breakpoint to be hit
    f.client.wait_for_event(has_thread_paused, timeout());

    let response = f.evaluate(123, thread_id, "z[0]");
    match response.payload {
        Some(debug_event::Payload::Error(e)) => {
            assert_eq!(e.message, "name 'z' is not defined")
        }
        other => panic!("expected an error, got {other:?}"),
    }
}

// b/143713658
#[test]
fn test_evaluate_request_resolves_globals_and_locals() {
    let mut f = Fixture::set_up_server_and_client();
    f.send_start_debugging_request();
    let file = "/a/build/file/foo.bzl";
    f.set_breakpoints(vec![location(file, 4, 0)]);

    let w = f.exec_in_worker_thread(
        file,
        "_global = [1,2,3]\n\ndef _func(my_arg):\n  pass\n\n_func(my_arg = [4,5,6])\n",
    );
    let thread_id = f.workers[w].id;

    // wait for breakpoint to be hit
    f.client.wait_for_event(has_thread_paused, timeout());

    let response_for_global = f.evaluate(123, thread_id, "_global[1]");
    let expected = f.get_value_proto("Evaluation result", |m| m.heap().alloc(2));
    match response_for_global.payload {
        Some(debug_event::Payload::Evaluate(r)) => assert_eq!(r.result, Some(expected)),
        other => panic!("expected Evaluate, got {other:?}"),
    }

    let response_for_local = f.evaluate(124, thread_id, "my_arg[1]");
    match response_for_local.payload {
        Some(debug_event::Payload::Error(e)) => {
            assert_eq!(e.message, "name 'my_arg' is not defined")
        }
        other => panic!("expected an error, got {other:?}"),
    }
}

const STEP_FILE: &str = "def fn():\n  a = 2\n  return a\nx = fn()\ny = [2,3,4]\n";

#[test]
fn test_step_into_function() {
    let mut f = Fixture::set_up_server_and_client();
    f.send_start_debugging_request();
    let file = "/a/build/file/test.bzl";
    f.set_breakpoints(vec![location(file, 4, 0)]);

    let w = f.exec_in_worker_thread(file, STEP_FILE);
    let thread_id = f.workers[w].id;

    // wait for breakpoint to be hit
    let event = f.client.wait_for_event(has_thread_paused, timeout()).unwrap();
    assert_eq!(
        paused_thread(&event).location.unwrap().line_number,
        4
    );

    f.client.clear_unnumbered_events();
    f.continue_execution(thread_id, Stepping::Into);
    let event = f.client.wait_for_event(has_thread_paused, timeout()).unwrap();

    // check we're paused inside the function
    assert_eq!(f.list_frames(thread_id).frame.len(), 2);

    // and verify the location and pause reason as well
    let paused = paused_thread(&event);
    assert_eq!(paused.pause_reason, PauseReason::Stepping as i32);
    assert_eq!(paused.location, Some(location(file, 2, 3)));
}

#[test]
fn test_step_over_function() {
    let mut f = Fixture::set_up_server_and_client();
    f.send_start_debugging_request();
    let file = "/a/build/file/test.bzl";
    f.set_breakpoints(vec![location(file, 4, 0)]);

    let w = f.exec_in_worker_thread(file, STEP_FILE);
    let thread_id = f.workers[w].id;

    // wait for breakpoint to be hit
    let event = f.client.wait_for_event(has_thread_paused, timeout()).unwrap();
    assert_eq!(
        paused_thread(&event).location.unwrap().line_number,
        4
    );

    f.client.clear_unnumbered_events();
    f.continue_execution(thread_id, Stepping::Over);
    let event = f.client.wait_for_event(has_thread_paused, timeout()).unwrap();

    let paused = paused_thread(&event);
    assert_eq!(paused.pause_reason, PauseReason::Stepping as i32);
    assert_eq!(paused.location, Some(location(file, 5, 1)));
}

#[test]
fn test_step_out_of_function() {
    let mut f = Fixture::set_up_server_and_client();
    f.send_start_debugging_request();
    let file = "/a/build/file/test.bzl";
    f.set_breakpoints(vec![location(file, 2, 0)]);

    let w = f.exec_in_worker_thread(file, STEP_FILE);
    let thread_id = f.workers[w].id;

    // wait for breakpoint to be hit
    f.client.wait_for_event(has_thread_paused, timeout());

    assert_eq!(f.list_frames(thread_id).frame.len(), 2);

    f.client.clear_unnumbered_events();
    f.continue_execution(thread_id, Stepping::Out);
    let event = f.client.wait_for_event(has_thread_paused, timeout()).unwrap();

    let paused = paused_thread(&event);
    assert_eq!(paused.pause_reason, PauseReason::Stepping as i32);
    assert_eq!(paused.location, Some(location(file, 5, 1)));
}
