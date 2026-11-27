//! `GrpcServerImpl`: the gRPC command server.
//!
//! This class is a little complicated and rich in multithreading, so an explanation of its
//! innards follows (Java's class comment, adapted).
//!
//! The gRPC handlers (`ping`, `cancel`, `updateTerminalSize`, and the start of `run`) run on the
//! tonic runtime's threads. For `run()`, we transfer the call to a thread of our own (Java's
//! `commandExecutorPool`). The command thread streams its output through a
//! [`BlockingStreamObserver`], which blocks while the client does not read (gRPC flow control),
//! so that the server never buffers an unbounded amount of outgoing data.
//!
//! In addition to these threads, we maintain one extra thread for handling the server timeout
//! and an interrupt watcher thread is started for each interrupt request that logs if it takes
//! too long to take effect.
//!
//! Each running RPC has a UUID associated with it that is used to identify it when a client
//! wants to cancel it. Cancellation is done by the client sending the server a `cancel()` RPC
//! call which results in the main thread of the command being interrupted.

use std::fmt;
use std::io::Write as _;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use blaze_events::io::{OutErr, OutputStream};
use blaze_events::ui::Clock;
use blaze_options::invocation_policy::InvocationPolicyParser;
use blaze_protos::JavaMessage;
use blaze_protos::command_server::{
    CancelRequest, CancelResponse, PingRequest, PingResponse, RunRequest, RunResponse, ServerInfo,
    TerminalSizeRequest, TerminalSizeResponse,
};
use blaze_protos::failure_details::failure_detail::Category;
use blaze_protos::failure_details::{
    Command, FailureDetail, Filesystem, GrpcServer, Interrupted, command, filesystem, grpc_server,
    interrupted,
};
use blaze_protos::google::protobuf::Any;
use blaze_shell::thread_interrupt;
use blaze_util::check_state;
use blaze_util::exit_code::{DetailedExitCode, ExitCode};
use blaze_util::string_encoding::{latin1_decode, latin1_encode_lossy};
use blaze_vfs::Path;
use parking_lot::Mutex;
use tokio::sync::{mpsc, oneshot};
use tonic::{Request, Response, Status};

use crate::blaze_command_result::BlazeCommandResult;
use crate::command_dispatcher::{
    CommandDispatcher, CommandExtensionReporter, IdleTaskResultsSupplier, LockingMode, UiVerbosity,
};
use crate::command_manager::CommandManager;
use crate::grpc::stubs::command_server_server;
use crate::grpc::{Server, ServerHandle, ServerShutdown};
use crate::pid_file_watcher::PidFileWatcher;
use crate::server_watcher_runnable::ServerWatcherRunnable;
use crate::shutdown_hooks::ShutdownHooks;
use crate::uncaught_exception;

/// An exception that should cause the server to exit at once with the given code.
///
/// PORT: belongs to `lib/util` (`AbruptExitException`); `blaze_runtime` has its own copy above
/// this crate. Same fields; the lead can make one re-export the other.
// Java: com/google/devtools/build/lib/util/AbruptExitException.java:28 AbruptExitException
#[derive(Clone, Debug, PartialEq)]
pub struct AbruptExitException {
    pub message: String,
    pub detailed_exit_code: DetailedExitCode,
}

impl AbruptExitException {
    pub fn new(detailed_exit_code: DetailedExitCode) -> Self {
        let message = detailed_exit_code
            .get_failure_detail()
            .map(|f| f.message.clone())
            .unwrap_or_default();
        AbruptExitException {
            message,
            detailed_exit_code,
        }
    }

    pub fn get_message(&self) -> &str {
        &self.message
    }

    pub fn get_detailed_exit_code(&self) -> &DetailedExitCode {
        &self.detailed_exit_code
    }

    pub fn get_exit_code(&self) -> ExitCode {
        self.detailed_exit_code.get_exit_code()
    }
}

impl fmt::Display for AbruptExitException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for AbruptExitException {}

// Java: com/google/devtools/build/lib/server/GrpcServerImpl.java:146 GrpcServerImpl.StreamType
#[allow(clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamType {
    STDOUT,
    STDERR,
}

/// A wrapper for a server-streaming response that blocks on [`BlockingStreamObserver::on_next`]
/// calls if the underlying stream is not ready.
///
/// It does not react to the interrupt flag in order to allow Bazel to complete the current
/// command while printing output as well as sending the final exit code to the client. However,
/// it maintains the interrupt flag if it is already set.
///
/// PORT: grpc-java's `ServerCallStreamObserver` (`isReady`, the on-ready handler) is a bounded
/// channel whose receiver is the response stream tonic polls; hyper polls the stream only when
/// the HTTP/2 stream has send capacity, so a sender blocked on the full channel is blocked on
/// the client's flow control. The wait (`block_on(send)`) does not watch the interrupt status,
/// as Java's loop ignores `InterruptedException`. "Cancelled" is the receiver having been
/// dropped (the client went away or the server shut down now).
// Java: com/google/devtools/build/lib/server/GrpcServerImpl.java:161 GrpcServerImpl.BlockingStreamObserver
pub struct BlockingStreamObserver<T> {
    /// `None` after `on_completed`.
    observer: Mutex<Option<mpsc::Sender<Result<T, Status>>>>,
}

/// The capacity of the channel between a command thread and its response stream: Java's
/// `isReady()` is false once grpc-java's per-stream outbound buffer (32 KiB) is full; here the
/// channel holds one message and tonic's encoder and the HTTP/2 stream window hold the rest.
const BLOCKING_STREAM_OBSERVER_CAPACITY: usize = 1;

impl<T: Send + 'static> BlockingStreamObserver<T> {
    /// A new observer and the response stream for tonic.
    pub fn new() -> (
        Arc<BlockingStreamObserver<T>>,
        tokio_stream::wrappers::ReceiverStream<Result<T, Status>>,
    ) {
        let (tx, rx) = mpsc::channel(BLOCKING_STREAM_OBSERVER_CAPACITY);
        (
            Arc::new(BlockingStreamObserver {
                observer: Mutex::new(Some(tx)),
            }),
            tokio_stream::wrappers::ReceiverStream::new(rx),
        )
    }

    /// `isReady()` of the underlying observer: a message can be sent without blocking.
    pub fn is_ready(&self) -> bool {
        match &*self.observer.lock() {
            Some(sender) => sender.capacity() > 0,
            None => false,
        }
    }

    /// `isCancelled()` of the underlying observer.
    pub fn is_cancelled(&self) -> bool {
        match &*self.observer.lock() {
            Some(sender) => sender.is_closed(),
            None => false,
        }
    }

    // Java: com/google/devtools/build/lib/server/GrpcServerImpl.java:177 BlockingStreamObserver.onNext
    pub fn on_next(&self, response: T) {
        // synchronized: one sender at a time, so messages keep the order of the calls.
        let observer = self.observer.lock();
        let Some(sender) = observer.as_ref() else {
            // grpc-java throws IllegalStateException("call already closed").
            panic!("IllegalStateException: call already closed");
        };
        // According to the documentation, if onNext is called in a canceled stream, it will be
        // silently ignored. The wait for the client is not interrupted (see the class comment).
        // PORT: `futures::executor::block_on` rather than tokio's `blocking_send`, which panics
        // when the writing thread happens to be a tokio runtime thread (a module's async code
        // writing to the command's OutErr); Java's wait has no such restriction.
        let sent = futures::executor::block_on(sender.send(Ok(response)));
        // Restore the interrupt bit if the stream was cancelled (the interrupt bit itself is
        // never cleared here).
        if sent.is_err() || sender.is_closed() {
            thread_interrupt::interrupt_current();
        }
    }

    // Java: com/google/devtools/build/lib/server/GrpcServerImpl.java:203 BlockingStreamObserver.onCompleted
    pub fn on_completed(&self) {
        // Dropping the sender ends the response stream; tonic then sends the OK status.
        self.observer.lock().take();
    }
}

/// Command extension reporter that packs the protobuf into a RunResponse and sends it.
// Java: com/google/devtools/build/lib/server/GrpcServerImpl.java:209 GrpcServerImpl.RpcCommandExtensionReporter
struct RpcCommandExtensionReporter {
    command_id: String,
    response_cookie: String,
    observer: Arc<BlockingStreamObserver<RunResponse>>,
    lock: Mutex<()>,
}

impl CommandExtensionReporter for RpcCommandExtensionReporter {
    // Java: com/google/devtools/build/lib/server/GrpcServerImpl.java:226 RpcCommandExtensionReporter.report
    fn report(&self, command_extension: Any) {
        let _synchronized = self.lock.lock();
        self.observer.on_next(RunResponse {
            cookie: self.response_cookie.clone(),
            command_id: self.command_id.clone(),
            standard_output: Vec::new(),
            command_extensions: vec![command_extension],
            ..Default::default()
        });
    }
}

/// An output stream that forwards the data written to it over the gRPC command stream.
// Java: com/google/devtools/build/lib/server/GrpcServerImpl.java:249 GrpcServerImpl.RpcOutputStream
struct RpcOutputStream {
    command_id: String,
    response_cookie: String,
    type_: StreamType,
    observer: Arc<BlockingStreamObserver<RunResponse>>,
}

// Java: com/google/devtools/build/lib/server/GrpcServerImpl.java:250 RpcOutputStream.CHUNK_SIZE
const CHUNK_SIZE: usize = 8192;

impl OutputStream for RpcOutputStream {
    // Java: com/google/devtools/build/lib/server/GrpcServerImpl.java:272 RpcOutputStream.write
    fn write(&self, b: &[u8]) -> std::io::Result<()> {
        for input in b.chunks(CHUNK_SIZE) {
            let mut response = RunResponse {
                cookie: self.response_cookie.clone(),
                command_id: self.command_id.clone(),
                ..Default::default()
            };
            match self.type_ {
                StreamType::STDOUT => response.standard_output = input.to_vec(),
                StreamType::STDERR => response.standard_error = input.to_vec(),
            }
            // This can block waiting for the client to read the available data.
            // PORT: Java catches a StatusRuntimeException here (bug report, then interrupt);
            // sending to the response stream cannot fail with one.
            self.observer.on_next(response);
        }
        Ok(())
    }
}

// These paths are all relative to the server directory
// Java: com/google/devtools/build/lib/server/GrpcServerImpl.java:309 GrpcServerImpl.PORT_FILE
const PORT_FILE: &str = "command_port";
// Java: com/google/devtools/build/lib/server/GrpcServerImpl.java:310 GrpcServerImpl.REQUEST_COOKIE_FILE
const REQUEST_COOKIE_FILE: &str = "request_cookie";
// Java: com/google/devtools/build/lib/server/GrpcServerImpl.java:311 GrpcServerImpl.RESPONSE_COOKIE_FILE
const RESPONSE_COOKIE_FILE: &str = "response_cookie";
// Java: com/google/devtools/build/lib/server/GrpcServerImpl.java:312 GrpcServerImpl.SERVER_INFO_FILE
const SERVER_INFO_FILE: &str = "server_info.rawproto";

/// gRPC server class.
// Java: com/google/devtools/build/lib/server/GrpcServerImpl.java:113 GrpcServerImpl
pub struct GrpcServerImpl {
    command_manager: Arc<CommandManager>,
    dispatcher: Arc<dyn CommandDispatcher>,
    shutdown_hooks: Arc<ShutdownHooks>,
    clock: Arc<dyn Clock>,
    server_directory: Path,
    request_cookie: String,
    response_cookie: String,
    max_idle_seconds: i32,
    shutdown_on_low_sys_mem: bool,
    pid_file_watcher: Arc<PidFileWatcher>,
    server_pid: i32,
    port: i32,

    server: Mutex<Option<ServerHandle>>,
    serving: AtomicBool,
    /// The `%d` of the `grpc-command-%d` thread names.
    command_thread_counter: AtomicU64,
}

impl GrpcServerImpl {
    // Java: com/google/devtools/build/lib/server/GrpcServerImpl.java:116 GrpcServerImpl.create
    pub fn create(
        dispatcher: Arc<dyn CommandDispatcher>,
        shutdown_hooks: Arc<ShutdownHooks>,
        pid_file_watcher: Arc<PidFileWatcher>,
        clock: Arc<dyn Clock>,
        port: i32,
        server_directory: Path,
        server_pid: i32,
        max_idle_seconds: i32,
        shutdown_on_low_sys_mem: bool,
        idle_server_tasks: bool,
        slow_interrupt_message_suffix: Option<String>,
    ) -> Arc<GrpcServerImpl> {
        GrpcServerImpl::new(
            dispatcher,
            shutdown_hooks,
            pid_file_watcher,
            clock,
            port,
            generate_cookie(16),
            generate_cookie(16),
            server_directory,
            server_pid,
            max_idle_seconds,
            shutdown_on_low_sys_mem,
            idle_server_tasks,
            slow_interrupt_message_suffix,
        )
    }

    // Java: com/google/devtools/build/lib/server/GrpcServerImpl.java:333 GrpcServerImpl.<init>
    pub fn new(
        dispatcher: Arc<dyn CommandDispatcher>,
        shutdown_hooks: Arc<ShutdownHooks>,
        pid_file_watcher: Arc<PidFileWatcher>,
        clock: Arc<dyn Clock>,
        port: i32,
        request_cookie: String,
        response_cookie: String,
        server_directory: Path,
        server_pid: i32,
        max_idle_seconds: i32,
        shutdown_on_low_sys_mem: bool,
        do_idle_server_tasks: bool,
        slow_interrupt_message_suffix: Option<String>,
    ) -> Arc<GrpcServerImpl> {
        Arc::new(GrpcServerImpl {
            dispatcher,
            shutdown_hooks,
            pid_file_watcher,
            clock,
            port,
            request_cookie,
            response_cookie,
            server_directory,
            server_pid,
            max_idle_seconds,
            shutdown_on_low_sys_mem,
            server: Mutex::new(None),
            serving: AtomicBool::new(false),
            command_thread_counter: AtomicU64::new(0),
            command_manager: CommandManager::new(
                do_idle_server_tasks,
                slow_interrupt_message_suffix,
            ),
        })
    }

    /// The service to add to a gRPC server (Java: `addService(this)`).
    pub fn service(self: &Arc<Self>) -> CommandServerService {
        CommandServerService(self.clone())
    }

    /// The request cookie (for tests and the probe).
    pub fn request_cookie(&self) -> &str {
        &self.request_cookie
    }

    /// `commandExecutorPool.execute(runnable)`: runs `runnable` on a fresh thread named
    /// `grpc-command-%d`.
    ///
    /// PORT: Java's pool is a cached thread pool (idle threads are reused); Blaze starts a fresh
    /// thread for each task, with at least the main thread's stack size, so that no
    /// thread-local state (the interrupt status, evaluator helpers) carries from one command to
    /// the next. A panic escaping the task is an uncaught exception
    /// ([`uncaught_exception::handle_uncaught_exception`]).
    fn command_executor_pool_execute(&self, runnable: impl FnOnce() + Send + 'static) {
        let n = self.command_thread_counter.fetch_add(1, Ordering::SeqCst);
        std::thread::Builder::new()
            .name(format!("grpc-command-{n}"))
            .stack_size(command_thread_stack_size())
            .spawn(move || uncaught_exception::run_catching(runnable))
            .expect("failed to start a grpc-command thread");
    }

    /// This is called when the server is shut down as a result of a "clean --expunge".
    ///
    /// In this case, no files should be deleted on shutdown hooks, since clean also deletes the
    /// lock file, and there is a small possibility of the following sequence of events:
    ///
    /// 1. Client 1 runs "blaze clean --expunge"
    /// 2. Client 2 runs a command and waits for client 1 to finish
    /// 3. The clean command deletes everything including the lock file
    /// 4. Client 2 starts running and since the output base is empty, starts up a new server,
    ///    which creates its own socket and PID files
    /// 5. The server used by client runs its shutdown hooks, deleting the PID files created by
    ///    the new server
    ///
    /// It also disables the "die when the PID file changes" handler so that it doesn't kill the
    /// server while the "clean --expunge" command is running.
    // Java: com/google/devtools/build/lib/server/GrpcServerImpl.java:404 GrpcServerImpl.prepareForAbruptShutdown
    pub fn prepare_for_abrupt_shutdown(&self) {
        self.shutdown_hooks.disable();
        self.pid_file_watcher.signal_shutdown();
    }

    /// Interrupts (cancels) in-flight commands.
    // Java: com/google/devtools/build/lib/server/GrpcServerImpl.java:410 GrpcServerImpl.interrupt
    pub fn interrupt(&self) {
        self.command_manager.interrupt_inflight_commands();
    }

    /// Starts serving and block until the shutdown command is received.
    // Java: com/google/devtools/build/lib/server/GrpcServerImpl.java:439 GrpcServerImpl.serve
    pub fn serve(self: &Arc<Self>) -> Result<(), AbruptExitException> {
        check_state!(!self.serving.load(Ordering::SeqCst));

        // For reasons only Apple knows, you cannot bind to IPv4-localhost when you run in a
        // sandbox that only allows loopback traffic, but binding to IPv6-localhost works fine.
        // This would however break on systems that don't support IPv6. So what we'll do is to
        // try to bind to IPv6 and if that fails, try again with IPv4.
        // PORT: Java first throws "ipv6 is not preferred on the system." when Netty's epoll
        // transport says IPv6 is not preferred (a Netty bug workaround); without Netty, a system
        // without IPv6 fails the bind below the same way. The macOS retries are skipped (macOS
        // only).
        // new InetSocketAddress("[::1]", port) throws for a port outside 0..=65535.
        let port = u16::try_from(self.port).unwrap_or_else(|_| {
            panic!("IllegalArgumentException: port out of range:{}", self.port)
        });
        let server = match bind_and_start(
            SocketAddr::new(Ipv6Addr::LOCALHOST.into(), port),
            self.service(),
        ) {
            Ok(server) => server,
            Err(ipv6_exception) => {
                match bind_and_start(
                    SocketAddr::new(Ipv4Addr::LOCALHOST.into(), port),
                    self.service(),
                ) {
                    Ok(server) => server,
                    Err(ipv4_exception) => {
                        return Err(AbruptExitException::new(DetailedExitCode::of(
                            create_failure_detail(
                                &format!(
                                    "gRPC server failed to bind to IPv4 and IPv6 localhosts on port {}: [IPv4] {}\n[IPv6] {}",
                                    self.port, ipv4_exception, ipv6_exception
                                ),
                                grpc_server::Code::ServerBindFailure,
                            ),
                        )));
                    }
                }
            }
        };
        *self.server.lock() = Some(server.handle());

        if self.max_idle_seconds > 0 {
            let watcher = ServerWatcherRunnable::new(
                Arc::new(server.handle()),
                self.max_idle_seconds as i64,
                self.shutdown_on_low_sys_mem,
                self.command_manager.clone(),
                self.clock.clone(),
            );
            std::thread::Builder::new()
                .name("grpc-timeout-and-memory".to_string())
                .spawn(move || uncaught_exception::run_catching(move || watcher.run()))
                .expect("failed to start the grpc-timeout-and-memory thread");
        }
        self.serving.store(true, Ordering::SeqCst);

        self.write_server_status_files(&server)?;

        server.await_termination();
        Ok(())
    }

    // Java: com/google/devtools/build/lib/server/GrpcServerImpl.java:501 GrpcServerImpl.writeServerStatusFiles
    fn write_server_status_files(&self, server: &Server) -> Result<(), AbruptExitException> {
        // InetAddresses.toUriString(address.getAddress()) + ":" + server.getPort()
        let address = server.get_address();
        let address_string = match address {
            SocketAddr::V6(a) => format!("[{}]:{}", a.ip(), server.get_port()),
            SocketAddr::V4(a) => format!("{}:{}", a.ip(), server.get_port()),
        };
        self.write_server_file(PORT_FILE, &address_string)?;
        self.write_server_file(REQUEST_COOKIE_FILE, &self.request_cookie)?;
        self.write_server_file(RESPONSE_COOKIE_FILE, &self.response_cookie)?;

        let info = ServerInfo {
            pid: self.server_pid,
            address: address_string,
            request_cookie: self.request_cookie.clone(),
            response_cookie: self.response_cookie.clone(),
        };

        // Write then mv so the user never sees incomplete contents.
        let server_info_tmp_file = self
            .server_directory
            .get_child(&format!("{SERVER_INFO_FILE}.tmp"));
        let written = (|| -> std::io::Result<()> {
            {
                let mut out = server_info_tmp_file.get_output_stream()?;
                out.write_all(&info.to_byte_array())?;
                out.flush()?;
            }
            let server_info_file = self.server_directory.get_child(SERVER_INFO_FILE);
            server_info_tmp_file.rename_to(&server_info_file)?;
            self.shutdown_hooks.delete_at_exit(server_info_file);
            Ok(())
        })();
        written.map_err(|e| {
            create_filesystem_failure_exception("Failed to write server info file", &e)
        })
    }

    // Java: com/google/devtools/build/lib/server/GrpcServerImpl.java:529 GrpcServerImpl.writeServerFile
    fn write_server_file(&self, name: &str, contents: &str) -> Result<(), AbruptExitException> {
        let file = self.server_directory.get_child(name);
        // FileSystemUtils.writeContentAsLatin1(file, contents)
        let written = (|| -> std::io::Result<()> {
            let mut out = file.get_output_stream()?;
            out.write_all(&latin1_encode_lossy(contents))?;
            out.flush()
        })();
        if let Err(e) = written {
            return Err(create_filesystem_failure_exception(
                &format!("Server file ({}) write failed", file.get_path_string()),
                &e,
            ));
        }
        self.shutdown_hooks.delete_at_exit(file);
        Ok(())
    }

    // Java: com/google/devtools/build/lib/server/GrpcServerImpl.java:539 GrpcServerImpl.executeCommand
    fn execute_command(
        self: &Arc<Self>,
        request: RunRequest,
        observer: Arc<BlockingStreamObserver<RunResponse>>,
    ) {
        let bad_cookie = !self.is_valid_request_cookie(&request.cookie);
        if bad_cookie || request.client_description.is_empty() {
            let failure_detail = if bad_cookie {
                create_failure_detail(
                    "Invalid RunRequest: bad cookie",
                    grpc_server::Code::BadCookie,
                )
            } else {
                create_failure_detail(
                    "Invalid RunRequest: no client description",
                    grpc_server::Code::NoClientDescription,
                )
            };
            observer.on_next(RunResponse {
                finished: true,
                exit_code: ExitCode::LOCAL_ENVIRONMENTAL_ERROR.get_numeric_exit_code(),
                failure_detail: Some(failure_detail),
                ..Default::default()
            });
            observer.on_completed();
            // PORT: skipped, logging of a StatusRuntimeException ("Client cancelled command
            // while rejecting it"): sending cannot throw.
            return;
        }

        let mut command_id: String;
        let result: BlazeCommandResult;

        // TODO(b/63925394): This information needs to be passed to the GotOptionsEvent, which
        // does not currently have the explicit startup options. See Improved Command Line
        // Reporting design doc for details.
        // Convert the startup options record to Java strings, source first.
        let mut startup_options: Vec<(String, String)> = Vec::new();
        for option in &request.startup_options {
            // UTF-8 won't do because we want to be able to pass arbitrary binary strings.
            startup_options.push((
                platform_bytes_to_internal_string(&option.source),
                platform_bytes_to_internal_string(&option.option),
            ));
        }

        self.command_manager.preempt_eligible_commands();

        {
            let mut command = if request.preemptible {
                self.command_manager.create_preemptible_command()
            } else {
                self.command_manager.create_command()
            };
            command_id = command.get_id().to_string();

            // Send the client the command id as soon as we know it.
            observer.on_next(RunResponse {
                cookie: self.response_cookie.clone(),
                command_id: command_id.clone(),
                ..Default::default()
            });
            // PORT: skipped, logging of a StatusRuntimeException ("The client cancelled the
            // command before receiving the command id"): sending cannot throw.

            let rpc_out_err = OutErr::create(
                Arc::new(RpcOutputStream {
                    command_id: command_id.clone(),
                    response_cookie: self.response_cookie.clone(),
                    type_: StreamType::STDOUT,
                    observer: observer.clone(),
                }),
                Arc::new(RpcOutputStream {
                    command_id: command_id.clone(),
                    response_cookie: self.response_cookie.clone(),
                    type_: StreamType::STDERR,
                    observer: observer.clone(),
                }),
            );

            // Transform args into Bazel's internal string representation.
            let args: Vec<String> = request
                .arg
                .iter()
                .map(|a| platform_bytes_to_internal_string(a))
                .collect();

            blaze_util::server_logger::info(
                "com.google.devtools.build.lib.server.GrpcServerImpl",
                "executeCommand",
                &blaze_util::server_logger::safe_request_log_string(&args),
            );
            let command_manager = self.command_manager.clone();
            let idle_task_results_supplier: IdleTaskResultsSupplier =
                Arc::new(move || command_manager.get_idle_task_results());
            let exec_result =
                match InvocationPolicyParser::parse_policy(Some(&request.invocation_policy)) {
                    Ok(policy) => self.dispatcher.exec_with_terminal_size_monitor(
                        policy,
                        args,
                        rpc_out_err.clone(),
                        if request.block_for_lock {
                            LockingMode::WAIT
                        } else {
                            LockingMode::ERROR_OUT
                        },
                        if request.quiet {
                            UiVerbosity::QUIET
                        } else {
                            UiVerbosity::NORMAL
                        },
                        request.client_description.clone(),
                        self.clock.current_time_millis(),
                        Some(startup_options),
                        idle_task_results_supplier,
                        request.command_extensions.clone(),
                        Arc::new(RpcCommandExtensionReporter {
                            command_id: command_id.clone(),
                            response_cookie: self.response_cookie.clone(),
                            observer: observer.clone(),
                            lock: Mutex::new(()),
                        }),
                        command.get_terminal_size_monitor(),
                    ),
                    // Java: com/google/devtools/build/lib/server/GrpcServerImpl.java:622 GrpcServerImpl.executeCommand
                    Err(e) => {
                        rpc_out_err.print_err_ln(e.get_message());
                        Ok(BlazeCommandResult::detailed_exit_code(
                            DetailedExitCode::of(FailureDetail {
                                message: format!(
                                    "Invocation policy parsing failed: {}",
                                    e.get_message()
                                ),
                                category: Some(Category::Command(Command {
                                    code: command::Code::InvocationPolicyParseFailure as i32,
                                })),
                            }),
                        ))
                    }
                };
            match exec_result {
                Ok(r) => {
                    // Record tasks to be run by IdleTaskManager. This is triggered when the
                    // RunningCommand is dropped (Java: closed), as we go out of scope
                    // immediately after this.
                    command.set_idle_tasks(r.get_idle_tasks().to_vec());
                    result = r;
                }
                Err(_interrupted) => {
                    result = BlazeCommandResult::detailed_exit_code(
                        interrupted_detailed_exit_code("Command dispatch interrupted"),
                    );
                    command_id = String::new(); // The default value, the client will ignore it
                }
            }
        }
        let mut response = RunResponse {
            cookie: self.response_cookie.clone(),
            command_id,
            finished: true,
            termination_expected: result.shutdown(),
            ..Default::default()
        };

        if let Some(exec_request) = result.get_exec_request() {
            response.exit_code = result.get_exit_code().get_numeric_exit_code();
            response.exec_request = Some(exec_request.clone());
            if let Some(failure_detail) = result.get_failure_detail() {
                response.failure_detail = Some(failure_detail.clone());
            }
        } else {
            response.exit_code = result.get_exit_code().get_numeric_exit_code();
            if let Some(failure_detail) = result.get_failure_detail() {
                response.failure_detail = Some(failure_detail.clone());
            }
        }

        response
            .command_extensions
            .extend(result.get_response_extensions().iter().cloned());
        observer.on_next(response);
        observer.on_completed();
        // PORT: skipped, logging of a StatusRuntimeException ("The client cancelled the command
        // before receiving the command id"): sending cannot throw.

        if result.shutdown() {
            match &*self.server.lock() {
                Some(server) => server.shutdown(),
                None => panic!("NullPointerException: server"),
            }
        }
    }

    // Java: com/google/devtools/build/lib/server/GrpcServerImpl.java:678 GrpcServerImpl.run
    fn run(self: &Arc<Self>, request: RunRequest) -> CommandServerRunStream {
        // Switch to our own threads so that the response stream can be polled while the command
        // runs (see the module comment).
        let (blocking_stream_observer, stream) = BlockingStreamObserver::new();
        let this = self.clone();
        self.command_executor_pool_execute(move || {
            this.execute_command(request, blocking_stream_observer)
        });
        stream
    }

    // Java: com/google/devtools/build/lib/server/GrpcServerImpl.java:689 GrpcServerImpl.ping
    fn ping(self: &Arc<Self>, ping_request: &PingRequest) -> PingResponse {
        let _command = self.command_manager.create_command();
        let mut response = PingResponse::default();
        if self.is_valid_request_cookie(&ping_request.cookie) {
            response.cookie = self.response_cookie.clone();
        }
        response
    }

    // Java: com/google/devtools/build/lib/server/GrpcServerImpl.java:702 GrpcServerImpl.cancel
    async fn cancel(self: &Arc<Self>, request: CancelRequest) -> Result<CancelResponse, Status> {
        blaze_util::server_logger::info(
            "com.google.devtools.build.lib.server.GrpcServerImpl",
            "cancel",
            &format!("Got CancelRequest for command id {}", request.command_id),
        );
        if !self.is_valid_request_cookie(&request.cookie) {
            return Err(completed_without_response());
        }

        // Actually performing the cancellation can result in some blocking which we don't want
        // to do on the dispatcher thread, instead offload to command pool.
        let (tx, rx) = oneshot::channel();
        let this = self.clone();
        self.command_executor_pool_execute(move || {
            let _ = tx.send(this.do_cancel(&request));
        });
        rx.await
            .map_err(|_| Status::internal("cancellation thread died"))
    }

    // Java: com/google/devtools/build/lib/server/GrpcServerImpl.java:715 GrpcServerImpl.doCancel
    fn do_cancel(self: &Arc<Self>, request: &CancelRequest) -> CancelResponse {
        self.command_manager.do_cancel(request);
        // PORT: skipped, logging of a StatusRuntimeException ("Client cancelled RPC of
        // cancellation request for %s").
        CancelResponse {
            cookie: self.response_cookie.clone(),
        }
    }

    // Java: com/google/devtools/build/lib/server/GrpcServerImpl.java:728 GrpcServerImpl.updateTerminalSize
    async fn update_terminal_size(
        self: &Arc<Self>,
        request: TerminalSizeRequest,
    ) -> Result<TerminalSizeResponse, Status> {
        blaze_util::server_logger::info(
            "com.google.devtools.build.lib.server.GrpcServerImpl",
            "updateTerminalSize",
            &format!(
                "Got TerminalSizeRequest for command id {}",
                request.command_id
            ),
        );
        if !self.is_valid_request_cookie(&request.cookie) {
            return Err(completed_without_response());
        }

        // Updating the monitor may notify listeners that redraw the UI, so keep it off the gRPC
        // dispatcher thread just like cancellation handling.
        let (tx, rx) = oneshot::channel();
        let this = self.clone();
        self.command_executor_pool_execute(move || {
            let _ = tx.send(this.do_update_terminal_size(&request));
        });
        rx.await
            .map_err(|_| Status::internal("terminal size thread died"))
    }

    // Java: com/google/devtools/build/lib/server/GrpcServerImpl.java:741 GrpcServerImpl.doUpdateTerminalSize
    fn do_update_terminal_size(&self, request: &TerminalSizeRequest) -> TerminalSizeResponse {
        self.command_manager.do_update_terminal_size(request);
        // PORT: skipped, logging of a StatusRuntimeException ("Client cancelled terminal size
        // update for command id %s").
        TerminalSizeResponse {
            cookie: self.response_cookie.clone(),
        }
    }

    /// Returns whether or not the provided cookie is valid for this server using a
    /// constant-time comparison in order to guard against timing attacks.
    // Java: com/google/devtools/build/lib/server/GrpcServerImpl.java:758 GrpcServerImpl.isValidRequestCookie
    fn is_valid_request_cookie(&self, incoming_request_cookie: &str) -> bool {
        // Note that cookie file was written as latin-1, so use that here.
        message_digest_is_equal(
            &latin1_encode_lossy(incoming_request_cookie),
            &latin1_encode_lossy(&self.request_cookie),
        )
    }
}

/// The response stream of `Run`.
pub type CommandServerRunStream =
    tokio_stream::wrappers::ReceiverStream<Result<RunResponse, Status>>;

/// The tonic service of a [`GrpcServerImpl`] (Java: the `CommandServerImplBase` methods).
#[derive(Clone)]
pub struct CommandServerService(Arc<GrpcServerImpl>);

#[tonic::async_trait]
impl command_server_server::CommandServer for CommandServerService {
    type RunStream = CommandServerRunStream;

    async fn run(&self, request: Request<RunRequest>) -> Result<Response<Self::RunStream>, Status> {
        Ok(Response::new(self.0.run(request.into_inner())))
    }

    async fn cancel(
        &self,
        request: Request<CancelRequest>,
    ) -> Result<Response<CancelResponse>, Status> {
        self.0.cancel(request.into_inner()).await.map(Response::new)
    }

    async fn update_terminal_size(
        &self,
        request: Request<TerminalSizeRequest>,
    ) -> Result<Response<TerminalSizeResponse>, Status> {
        self.0
            .update_terminal_size(request.into_inner())
            .await
            .map(Response::new)
    }

    async fn ping(&self, request: Request<PingRequest>) -> Result<Response<PingResponse>, Status> {
        let this = self.0.clone();
        let request = request.into_inner();
        // Java runs ping on the gRPC thread; registering the command may wait for idle tasks to
        // stop (CommandManager.busy), so tell tokio this thread blocks.
        let response = tokio::task::block_in_place(move || this.ping(&request));
        Ok(Response::new(response))
    }
}

/// A unary call whose Java handler calls `onCompleted()` without `onNext()`. grpc-java 1.66.0 (the
/// pin's `io.grpc:grpc-core`, read from the bytecode of the oracle's `A-server.jar`) does not
/// close such a call with OK: `ServerCallImpl.closeInternal` sees a unary method that sent no
/// message and calls `handleInternalError(Status.INTERNAL.withDescription("Completed without a
/// response").asRuntimeException())`, which cancels the stream; `NettyServerStream.Sink.cancel`
/// enqueues `CancelServerStreamCommand.withReset`, so the peer gets `RST_STREAM` with the HTTP/2
/// error code `CANCEL`, which a gRPC client reports as status `CANCELLED`.
///
/// PORT: tonic cannot reset the stream from a handler; it answers with a trailers-only response
/// whose `grpc-status` is `CANCELLED` (1) and no message (a reset carries none).
// Java: io/grpc/internal/ServerCallImpl.java ServerCallImpl.closeInternal
fn completed_without_response() -> Status {
    Status::new(tonic::Code::Cancelled, "")
}

fn bind_and_start(address: SocketAddr, service: CommandServerService) -> std::io::Result<Server> {
    let listener = TcpListener::bind(address)?;
    Server::start(listener, service)
}

// Java: com/google/devtools/build/lib/server/GrpcServerImpl.java:374 GrpcServerImpl.generateCookie
fn generate_cookie(byte_count: usize) -> String {
    // SecureRandom.nextBytes
    let mut bytes = vec![0u8; byte_count];
    let mut filled = 0;
    while filled < byte_count {
        // SAFETY: writes at most `byte_count - filled` bytes into the buffer's tail.
        let n =
            unsafe { libc::getrandom(bytes[filled..].as_mut_ptr().cast(), byte_count - filled, 0) };
        if n < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            panic!("SecureRandom failed: {e}");
        }
        filled += n as usize;
    }
    let mut result = String::new();
    for b in bytes {
        // Integer.toHexString(b + 128), b a signed byte: 0..=255 without padding.
        result.push_str(&format!("{:x}", (b as i8 as i32) + 128));
    }
    result
}

/// `MessageDigest.isEqual`: a comparison whose time does not depend on where the inputs
/// differ.
fn message_digest_is_equal(digesta: &[u8], digestb: &[u8]) -> bool {
    if digesta.len() != digestb.len() {
        return false;
    }
    let mut result = 0u8;
    for (a, b) in digesta.iter().zip(digestb) {
        result |= a ^ b;
    }
    result == 0
}

// Java: com/google/devtools/build/lib/util/InterruptedFailureDetails.java:30 InterruptedFailureDetails.detailedExitCode
fn interrupted_detailed_exit_code(message: &str) -> DetailedExitCode {
    DetailedExitCode::of(FailureDetail {
        message: message.to_string(),
        category: Some(Category::Interrupted(Interrupted {
            code: interrupted::Code::Interrupted as i32,
        })),
    })
}

// Java: com/google/devtools/build/lib/server/GrpcServerImpl.java:765 GrpcServerImpl.createFilesystemFailureException
fn create_filesystem_failure_exception(message: &str, e: &std::io::Error) -> AbruptExitException {
    let e_message = e.to_string();
    AbruptExitException::new(DetailedExitCode::of(FailureDetail {
        message: format!(
            "{}{}",
            message,
            if e_message.is_empty() {
                String::new()
            } else {
                format!(": {e_message}")
            }
        ),
        category: Some(Category::Filesystem(Filesystem {
            code: filesystem::Code::ServerFileWriteFailure as i32,
        })),
    }))
}

// Java: com/google/devtools/build/lib/server/GrpcServerImpl.java:777 GrpcServerImpl.createFailureDetail
fn create_failure_detail(message: &str, detailed_code: grpc_server::Code) -> FailureDetail {
    FailureDetail {
        message: message.to_string(),
        category: Some(Category::GrpcServer(GrpcServer {
            code: detailed_code as i32,
        })),
    }
}

// Java: com/google/devtools/build/lib/server/GrpcServerImpl.java:784 GrpcServerImpl.platformBytesToInternalString
fn platform_bytes_to_internal_string(bytes: &[u8]) -> String {
    latin1_decode(bytes).into_owned()
}

/// The stack size of a `grpc-command-%d` thread: at least the main thread's (`RLIMIT_STACK`,
/// 8 MiB by default), since a command runs what the batch-mode main thread would run.
///
/// PORT: Blaze only (JVM threads get `-Xss`). With an unlimited `RLIMIT_STACK`, 256 MiB.
fn command_thread_stack_size() -> usize {
    const MIN: usize = 8 << 20;
    const UNLIMITED: usize = 256 << 20;
    let mut rlimit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: getrlimit writes into the struct.
    if unsafe { libc::getrlimit(libc::RLIMIT_STACK, &mut rlimit) } != 0 {
        return MIN;
    }
    if rlimit.rlim_cur == libc::RLIM_INFINITY {
        return UNLIMITED;
    }
    (rlimit.rlim_cur as usize).max(MIN)
}
