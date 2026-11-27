//! Port of `StarlarkDebugServer`: the debug server, its client listener thread and the request
//! handlers.

use std::io;
use std::net::TcpListener;
use std::sync::Arc;

use blaze_events::{Event, EventHandler};
use blaze_protos::starlark_debugging::{
    ContinueExecutionRequest, DebugEvent, DebugRequest, EvaluateRequest, GetChildrenRequest,
    ListFramesRequest, PauseThreadRequest, SetBreakpointsRequest, Stepping as SteppingProto,
    debug_request::Payload,
};
use blaze_starlark::debug::{DebugLocation, Debugger};
use starlark::eval::Evaluator;

use crate::event_helper;
use crate::thread_handler::{DebugRequestException, ThreadHandler};
use crate::transport::{DebugServerTransport, ListenPortCallback};

/// Callbacks at the debugging lifecycle's steps.
// Java: com/google/devtools/build/lib/starlarkdebug/server/StarlarkDebugServer.java:265 StarlarkDebugServer.DebugCallback
pub trait DebugCallback: Send + Sync {
    /// Called with the paths that have breakpoints when the client starts debugging.
    // Java: com/google/devtools/build/lib/starlarkdebug/server/StarlarkDebugServer.java:271 StarlarkDebugServer.DebugCallback.beforeDebuggingStart
    fn before_debugging_start(&self, _break_point_paths: &[String]) {}

    /// Blocks until debugging may start, if the callback requires it.
    // Java: com/google/devtools/build/lib/starlarkdebug/server/StarlarkDebugServer.java:273 StarlarkDebugServer.DebugCallback.maybeBlockBeforeStart
    fn maybe_block_before_start(&self) {}

    // Java: com/google/devtools/build/lib/starlarkdebug/server/StarlarkDebugServer.java:275 StarlarkDebugServer.DebugCallback.onClose
    fn on_close(&self) {}
}

/// `DebugCallback.noop()`.
// Java: com/google/devtools/build/lib/starlarkdebug/server/StarlarkDebugServer.java:266 StarlarkDebugServer.DebugCallback.noop
pub struct NoopCallback;

impl DebugCallback for NoopCallback {}

/// Manages the network socket and debugging state for threads running Starlark code.
// Java: com/google/devtools/build/lib/starlarkdebug/server/StarlarkDebugServer.java:46 StarlarkDebugServer
pub struct StarlarkDebugServer {
    event_handler: Arc<dyn EventHandler>,
    callback: Arc<dyn DebugCallback>,
    thread_handler: ThreadHandler,
    transport: DebugServerTransport,
    verbose_logging: bool,
}

impl StarlarkDebugServer {
    /// Initializes debugging support, setting up any debugging-specific overrides, then opens
    /// the debug server socket and blocks waiting for an incoming connection.
    // Java: com/google/devtools/build/lib/starlarkdebug/server/StarlarkDebugServer.java:59 StarlarkDebugServer.createAndWaitForConnection
    pub fn create_and_wait_for_connection(
        event_handler: Arc<dyn EventHandler>,
        port: u16,
        verbose_logging: bool,
        callback: Arc<dyn DebugCallback>,
    ) -> io::Result<Arc<StarlarkDebugServer>> {
        // new ServerSocket(port, /* backlog */ 1): the wildcard address.
        let server_socket = TcpListener::bind(("0.0.0.0", port))?;
        Self::create_and_wait_for_connection_on(
            event_handler,
            server_socket,
            verbose_logging,
            callback,
            None,
        )
    }

    // Java: com/google/devtools/build/lib/starlarkdebug/server/StarlarkDebugServer.java:68 StarlarkDebugServer.createAndWaitForConnection
    pub fn create_and_wait_for_connection_on(
        event_handler: Arc<dyn EventHandler>,
        server_socket: TcpListener,
        verbose_logging: bool,
        callback: Arc<dyn DebugCallback>,
        on_listen_port: Option<&ListenPortCallback>,
    ) -> io::Result<Arc<StarlarkDebugServer>> {
        let transport = DebugServerTransport::create_and_wait_for_client(
            event_handler.clone(),
            server_socket,
            verbose_logging,
            on_listen_port,
        )?;
        let server = Arc::new(StarlarkDebugServer {
            event_handler,
            callback,
            thread_handler: ThreadHandler::default(),
            transport,
            verbose_logging,
        });
        Self::listen_for_client_requests(server.clone());
        Ok(server)
    }

    /// Starts a worker thread listening for and responding to client debug requests.
    // Java: com/google/devtools/build/lib/starlarkdebug/server/StarlarkDebugServer.java:96 StarlarkDebugServer.listenForClientRequests
    fn listen_for_client_requests(server: Arc<StarlarkDebugServer>) {
        std::thread::Builder::new()
            .name("starlark-debug-server".to_owned())
            .spawn(move || {
                loop {
                    let Some(request) = server.transport.read_client_request() else {
                        break;
                    };
                    if let Some(response) = server.handle_client_request(&request) {
                        server.transport.post_event(&response);
                    }
                }
                server.event_handler.handle(&Event::info(
                    "Debug server listener thread closed; shutting down debug server and \
                     resuming all threads",
                ));
                server.close_server();
            })
            .expect("OutOfMemoryError: unable to create native thread");
    }

    // Java: com/google/devtools/build/lib/starlarkdebug/server/StarlarkDebugServer.java:130 StarlarkDebugServer.close
    fn close_server(&self) {
        if self.verbose_logging {
            self.event_handler
                .handle(&Event::debug("Closing debug server"));
        }
        if let Err(e) = self.transport.close() {
            self.event_handler.handle(&Event::error(format!(
                "Error shutting down the debug server: {e}"
            )));
        }
        self.callback.on_close();
        self.thread_handler.resume_all_threads();
    }

    /// Handles a request from the client, and returns the response, where relevant.
    // Java: com/google/devtools/build/lib/starlarkdebug/server/StarlarkDebugServer.java:159 StarlarkDebugServer.handleClientRequest
    pub fn handle_client_request(&self, request: &DebugRequest) -> Option<DebugEvent> {
        let sequence_number = request.sequence_number;
        let result = match &request.payload {
            Some(Payload::StartDebugging(_)) => {
                self.callback
                    .before_debugging_start(&self.thread_handler.get_breakpoint_file_paths());
                self.thread_handler.resume_all_threads();
                Ok(event_helper::start_debugging_response(sequence_number))
            }
            Some(Payload::ListFrames(r)) => self.list_frames(sequence_number, r),
            Some(Payload::SetBreakpoints(r)) => Ok(self.set_breakpoints(sequence_number, r)),
            Some(Payload::ContinueExecution(r)) => self.continue_execution(sequence_number, r),
            Some(Payload::PauseThread(r)) => self.pause_thread(sequence_number, r),
            Some(Payload::Evaluate(r)) => self.evaluate(sequence_number, r),
            Some(Payload::GetChildren(r)) => self.get_children(sequence_number, r),
            None => {
                // DebugEventHelper.error(sequenceNumber, "No request payload found") is built
                // and dropped; the unhandled-type error follows.
                Ok(event_helper::error_with(
                    sequence_number,
                    "Unhandled request type: PAYLOAD_NOT_SET",
                ))
            }
        };
        Some(result.unwrap_or_else(|e| event_helper::error_with(sequence_number, &e.0)))
    }

    // Java: com/google/devtools/build/lib/starlarkdebug/server/StarlarkDebugServer.java:189 StarlarkDebugServer.listFrames
    fn list_frames(
        &self,
        sequence_number: i64,
        request: &ListFramesRequest,
    ) -> Result<DebugEvent, DebugRequestException> {
        let frames = self.thread_handler.list_frames(request.thread_id)?;
        Ok(event_helper::list_frames_response(sequence_number, frames))
    }

    // Java: com/google/devtools/build/lib/starlarkdebug/server/StarlarkDebugServer.java:196 StarlarkDebugServer.setBreakpoints
    fn set_breakpoints(&self, sequence_number: i64, request: &SetBreakpointsRequest) -> DebugEvent {
        self.thread_handler.set_breakpoints(&request.breakpoint);
        event_helper::set_breakpoints_response(sequence_number)
    }

    // Java: com/google/devtools/build/lib/starlarkdebug/server/StarlarkDebugServer.java:202 StarlarkDebugServer.evaluate
    fn evaluate(
        &self,
        sequence_number: i64,
        request: &EvaluateRequest,
    ) -> Result<DebugEvent, DebugRequestException> {
        let value = self
            .thread_handler
            .evaluate(request.thread_id, &request.statement)?;
        Ok(event_helper::evaluate_response(sequence_number, value))
    }

    // Java: com/google/devtools/build/lib/starlarkdebug/server/StarlarkDebugServer.java:210 StarlarkDebugServer.getChildren
    fn get_children(
        &self,
        sequence_number: i64,
        request: &GetChildrenRequest,
    ) -> Result<DebugEvent, DebugRequestException> {
        let children = self
            .thread_handler
            .get_children_for_value(request.thread_id, request.value_id)?;
        Ok(event_helper::get_children_response(
            sequence_number,
            children,
        ))
    }

    // Java: com/google/devtools/build/lib/starlarkdebug/server/StarlarkDebugServer.java:219 StarlarkDebugServer.continueExecution
    fn continue_execution(
        &self,
        sequence_number: i64,
        request: &ContinueExecutionRequest,
    ) -> Result<DebugEvent, DebugRequestException> {
        let thread_id = request.thread_id;
        if thread_id == 0 {
            self.thread_handler.resume_all_threads();
            return Ok(event_helper::continue_execution_response(sequence_number));
        }
        let stepping = SteppingProto::try_from(request.stepping).unwrap_or(SteppingProto::None);
        self.thread_handler.resume_thread(thread_id, stepping)?;
        Ok(event_helper::continue_execution_response(sequence_number))
    }

    // Java: com/google/devtools/build/lib/starlarkdebug/server/StarlarkDebugServer.java:232 StarlarkDebugServer.pauseThread
    fn pause_thread(
        &self,
        sequence_number: i64,
        request: &PauseThreadRequest,
    ) -> Result<DebugEvent, DebugRequestException> {
        let thread_id = request.thread_id;
        if thread_id == 0 {
            self.thread_handler.pause_all_threads();
        } else {
            self.thread_handler.pause_thread(thread_id)?;
        }
        Ok(event_helper::pause_thread_response(sequence_number))
    }
}

impl Debugger for StarlarkDebugServer {
    // Java: com/google/devtools/build/lib/starlarkdebug/server/StarlarkDebugServer.java:152 StarlarkDebugServer.before
    fn before(&self, eval: &mut Evaluator<'_, '_, '_>, location: &DebugLocation) {
        if !self.transport.is_closed() {
            self.thread_handler
                .pause_if_necessary(eval, location, &self.transport);
        }
    }

    // Java: com/google/devtools/build/lib/starlarkdebug/server/StarlarkDebugServer.java:130 StarlarkDebugServer.close
    fn close(&self) {
        self.close_server();
    }
}
