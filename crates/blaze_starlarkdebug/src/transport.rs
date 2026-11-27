//! Port of `DebugServerTransport`: the socket between the debug server and its one client,
//! carrying length-delimited `DebugRequest` / `DebugEvent` protos.

use std::io::{self, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use blaze_events::{Event, EventHandler};
use blaze_protos::starlark_debugging::{DebugEvent, DebugRequest};
use parking_lot::Mutex;
use prost::Message;

use crate::event_helper;

/// Called with the listening port before the server waits for its client (tests).
pub type ListenPortCallback = Box<dyn Fn(u16) + Send + Sync>;

// Java: com/google/devtools/build/lib/starlarkdebug/server/DebugServerTransport.java:38 DebugServerTransport.onListenPortCallbackForTests
static ON_LISTEN_PORT_CALLBACK_FOR_TESTS: parking_lot::RwLock<Option<ListenPortCallback>> =
    parking_lot::RwLock::new(None);

/// Sets `DebugServerTransport.onListenPortCallbackForTests` (`None`: Java's no-op default).
pub fn set_on_listen_port_callback_for_tests(callback: Option<ListenPortCallback>) {
    *ON_LISTEN_PORT_CALLBACK_FOR_TESTS.write() = callback;
}

/// Manages the connection to and communication with the remote debug client.
// Java: com/google/devtools/build/lib/starlarkdebug/server/DebugServerTransport.java:34 DebugServerTransport
pub struct DebugServerTransport {
    event_handler: Arc<dyn EventHandler>,
    client_socket: TcpStream,
    request_stream: Mutex<BufReader<TcpStream>>,
    event_stream: Mutex<TcpStream>,
    verbose_logging: bool,
    closed: AtomicBool,
}

impl DebugServerTransport {
    /// Sets up the server transport and blocks while waiting for an incoming connection.
    // Java: com/google/devtools/build/lib/starlarkdebug/server/DebugServerTransport.java:41 DebugServerTransport.createAndWaitForClient
    pub fn create_and_wait_for_client(
        event_handler: Arc<dyn EventHandler>,
        server_socket: TcpListener,
        verbose_logging: bool,
    ) -> io::Result<DebugServerTransport> {
        // TODO(bazel-team): reject remote connections when running in a remote environment?
        event_handler.handle(&Event::progress("Waiting for debugger..."));
        if let Some(callback) = &*ON_LISTEN_PORT_CALLBACK_FOR_TESTS.read() {
            callback(server_socket.local_addr()?.port());
        }
        let (client_socket, _) = server_socket.accept()?;
        // ServerSocket.close() after the one connection: nothing else is accepted.
        drop(server_socket);
        event_handler.handle(&Event::info(
            "Debugger connection successfully established.",
        ));
        Ok(DebugServerTransport {
            event_handler,
            request_stream: Mutex::new(BufReader::new(client_socket.try_clone()?)),
            event_stream: Mutex::new(client_socket.try_clone()?),
            client_socket,
            verbose_logging,
            closed: AtomicBool::new(false),
        })
    }

    /// Blocks waiting for a properly-formed client request. Returns `None` if the client
    /// connection is closed.
    // Java: com/google/devtools/build/lib/starlarkdebug/server/DebugServerTransport.java:87 DebugServerTransport.readClientRequest
    pub fn read_client_request(&self) -> Option<DebugRequest> {
        let mut stream = self.request_stream.lock();
        match read_delimited(&mut *stream) {
            Ok(Some(request)) => {
                if self.verbose_logging {
                    self.event_handler.handle(&Event::debug(format!(
                        "Received debug client request:\n{}",
                        crate::text_format(&request)
                    )));
                }
                Some(request)
            }
            // parseDelimitedFrom returns null at the end of the stream.
            Ok(None) => None,
            Err(e) => {
                self.handle_parsing_error(&e);
                None
            }
        }
    }

    // Java: com/google/devtools/build/lib/starlarkdebug/server/DebugServerTransport.java:96 DebugServerTransport.handleParsingError
    fn handle_parsing_error(&self, e: &io::Error) {
        if self.is_closed() {
            // an IOException is expected when the client disconnects -- no need to log an error
            return;
        }
        let message = format!("Error parsing debug request: {e}");
        self.post_event(&event_helper::error(&message));
        self.event_handler.handle(&Event::error(message));
    }

    /// Posts a debug event.
    // Java: com/google/devtools/build/lib/starlarkdebug/server/DebugServerTransport.java:108 DebugServerTransport.postEvent
    pub fn post_event(&self, event: &DebugEvent) {
        if self.verbose_logging {
            self.event_handler.handle(&Event::debug(format!(
                "Sending debug event:\n{}",
                crate::text_format(event)
            )));
        }
        let mut stream = self.event_stream.lock();
        let bytes = event.encode_length_delimited_to_vec();
        if let Err(e) = stream.write_all(&bytes).and_then(|()| stream.flush()) {
            self.event_handler.handle(&Event::error(format!(
                "Failed to send debug event to client: {e}"
            )));
        }
    }

    // Java: com/google/devtools/build/lib/starlarkdebug/server/DebugServerTransport.java:126 DebugServerTransport.close
    pub fn close(&self) -> io::Result<()> {
        self.closed.store(true, Ordering::SeqCst);
        self.client_socket.shutdown(std::net::Shutdown::Both)
    }

    // Java: com/google/devtools/build/lib/starlarkdebug/server/DebugServerTransport.java:131 DebugServerTransport.isClosed
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }
}

/// `Message.parseDelimitedFrom`: a varint length and that many bytes; `None` at the end of the
/// stream.
pub fn read_delimited<M: Message + Default>(stream: &mut impl Read) -> io::Result<Option<M>> {
    let mut len: u64 = 0;
    let mut shift = 0;
    loop {
        let mut byte = [0u8; 1];
        match stream.read(&mut byte)? {
            0 if shift == 0 => return Ok(None),
            0 => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "truncated message",
                ));
            }
            _ => {}
        }
        len |= u64::from(byte[0] & 0x7f) << shift;
        if byte[0] & 0x80 == 0 {
            break;
        }
        shift += 7;
        if shift > 63 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "CodedInputStream encountered a malformed varint.",
            ));
        }
    }
    let mut buffer = vec![0u8; len as usize];
    stream.read_exact(&mut buffer)?;
    M::decode(buffer.as_slice())
        .map(Some)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
}
