//! The client stub of the `command_server.CommandServer` gRPC service
//! (`src/main/protobuf/command_server.proto:266`), which the C++ client gets from the generated
//! `command_server.grpc.pb.h` (`CommandServer::Stub`).
//!
//! Hand-written on `tonic::client::Grpc` with the prost message types of
//! `blaze_protos::command_server` (the same shape `tonic_build::manual` generates for
//! `blaze_remote`). Plain HTTP/2 (h2c, prior knowledge) on loopback; the service path is
//! `/command_server.CommandServer/<Method>`.
//!
//! The rest of the client is blocking code structured like the C++: [`StubRuntime`] owns the
//! tokio runtime, and every RPC is a blocking call (`block_on`) or, for `Run`, a stream whose
//! messages are read one at a time like `grpc::ClientReader::Read`.

use std::time::Duration;

use blaze_protos::command_server::{
    CancelRequest, CancelResponse, PingRequest, PingResponse, RunRequest, RunResponse,
    TerminalSizeRequest, TerminalSizeResponse,
};
use tonic::codegen::http::uri::PathAndQuery;
use tonic::transport::{Channel, Endpoint};

/// A finished RPC's `grpc::Status`: the numeric status code (`status.error_code()`, which the
/// C++ streams as an integer) and the message (`status.error_message()`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrpcStatus {
    pub code: i32,
    pub message: String,
}

impl GrpcStatus {
    /// `grpc::Status::OK`.
    pub fn ok() -> Self {
        GrpcStatus {
            code: 0,
            message: String::new(),
        }
    }

    /// `status.ok()`.
    pub fn is_ok(&self) -> bool {
        self.code == 0
    }

    fn from_tonic(status: &tonic::Status) -> Self {
        if std::env::var_os("BLAZE_TMP_GRPC_DEBUG").is_some() {
            let mut src: Option<&(dyn std::error::Error + 'static)> =
                std::error::Error::source(status);
            eprintln!("TMPDEBUG status {:?}", status);
            while let Some(e) = src {
                eprintln!("TMPDEBUG source {:?} / {}", e, e);
                src = e.source();
            }
        }
        if Self::is_connection_closed(status) {
            // PORT: a call whose connection was closed by the peer (the server process died)
            // ends in grpc-c++ with UNAVAILABLE (14) and the transport's text "Socket closed"
            // (what Bazel's client prints in "Server terminated abruptly (error code: 14, error
            // message: 'Socket closed', ...)"). tonic reports the same event as UNKNOWN with
            // hyper's text; h2 records the end of the connection as an I/O error of kind
            // BrokenPipe (or UnexpectedEof). Other transport failures keep tonic's status.
            return GrpcStatus {
                code: tonic::Code::Unavailable as i32,
                message: "Socket closed".to_string(),
            };
        }
        GrpcStatus {
            code: status.code() as i32,
            message: status.message().to_string(),
        }
    }

    /// Whether the call failed because the peer closed the connection (EOF on the socket).
    fn is_connection_closed(status: &tonic::Status) -> bool {
        let mut source: Option<&(dyn std::error::Error + 'static)> =
            std::error::Error::source(status);
        while let Some(e) = source {
            if let Some(io) = e.downcast_ref::<std::io::Error>() {
                return matches!(
                    io.kind(),
                    std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::UnexpectedEof
                );
            }
            source = e.source();
        }
        false
    }

    /// The status of a deadline that expired on the client.
    // PORT: grpc-c++ reports an expired client deadline as DEADLINE_EXCEEDED (4) with the
    // message "Deadline Exceeded"; tonic has no client deadline of its own (the
    // `grpc-timeout` header is sent, the wait is a tokio timeout), so the status is built here
    // with grpc-c++'s code and text.
    fn deadline_exceeded() -> Self {
        GrpcStatus {
            code: tonic::Code::DeadlineExceeded as i32,
            message: "Deadline Exceeded".to_string(),
        }
    }
}

/// The tokio runtime the gRPC calls of one `BlazeServer` run on.
///
/// PORT: grpc-c++ runs its own completion-queue threads; tonic needs a tokio runtime. A
/// multi-threaded runtime with one worker is used (not a current-thread one) because two
/// threads call into it at once, as in the C++: the main thread reading the `Run` stream and
/// the cancel thread sending `Cancel` / `UpdateTerminalSize`.
pub struct StubRuntime {
    rt: tokio::runtime::Runtime,
}

impl StubRuntime {
    pub fn new() -> std::io::Result<Self> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("blaze-client-grpc")
            .enable_all()
            .build()?;
        Ok(StubRuntime { rt })
    }

    pub fn block_on<F: std::future::Future>(&self, f: F) -> F::Output {
        self.rt.block_on(f)
    }

    pub fn spawn<F>(&self, f: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: std::future::Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.rt.spawn(f)
    }
}

/// `CommandServer::Stub` on a channel to one server address.
#[derive(Clone)]
pub struct CommandServerStub {
    inner: tonic::client::Grpc<Channel>,
}

/// `grpc::CreateCustomChannel(address, grpc::InsecureChannelCredentials(), channel_args)`
/// followed by `CommandServer::NewStub(channel)`.
///
/// The channel is lazy, as a grpc-c++ channel is: the connection is made by the first call
/// (`Ping` in `TryConnect`), whose deadline then covers the connection attempt too.
///
/// PORT: `GRPC_ARG_ENABLE_HTTP_PROXY = 0` has no counterpart: tonic's connector never consults
/// proxy settings, so proxies are always off, as the C++ asks.
pub fn new_stub(rt: &StubRuntime, address: &str) -> Option<CommandServerStub> {
    // The channel's background task is spawned on the runtime.
    let _guard = rt.rt.enter();
    // The address is "127.0.0.1:<port>", "[::1]:<port>" or "[0:0:0:0:0:0:0:1]:<port>"
    // (checked by BlazeServer::Connect); gRPC's target syntax takes it as host:port.
    let endpoint = Endpoint::from_shared(format!("http://{}", address)).ok()?;
    let channel = endpoint.connect_lazy();
    Some(CommandServerStub {
        inner: tonic::client::Grpc::new(channel),
    })
}

impl CommandServerStub {
    async fn ready(&mut self) -> Result<(), tonic::Status> {
        self.inner
            .ready()
            .await
            .map_err(|e| tonic::Status::unavailable(format!("{}", e)))
    }

    async fn unary<Req, Resp>(
        &mut self,
        method: &'static str,
        request: Req,
        deadline: Option<Duration>,
    ) -> Result<Resp, GrpcStatus>
    where
        Req: prost::Message + Send + Sync + 'static,
        Resp: prost::Message + Default + Send + Sync + 'static,
    {
        let call = async {
            self.ready().await?;
            let codec = tonic_prost::ProstCodec::<Req, Resp>::default();
            let mut req = tonic::Request::new(request);
            if let Some(d) = deadline {
                req.set_timeout(d);
            }
            self.inner
                .unary(req, PathAndQuery::from_static(method), codec)
                .await
                .map(tonic::Response::into_inner)
        };
        match deadline {
            Some(d) => match tokio::time::timeout(d, call).await {
                Ok(r) => r.map_err(|s| GrpcStatus::from_tonic(&s)),
                Err(_) => Err(GrpcStatus::deadline_exceeded()),
            },
            None => call.await.map_err(|s| GrpcStatus::from_tonic(&s)),
        }
    }

    /// `rpc Ping(PingRequest) returns (PingResponse)`.
    pub async fn ping(
        &mut self,
        request: PingRequest,
        deadline: Duration,
    ) -> Result<PingResponse, GrpcStatus> {
        self.unary(
            "/command_server.CommandServer/Ping",
            request,
            Some(deadline),
        )
        .await
    }

    /// `rpc Cancel(CancelRequest) returns (CancelResponse)`.
    pub async fn cancel(
        &mut self,
        request: CancelRequest,
        deadline: Duration,
    ) -> Result<CancelResponse, GrpcStatus> {
        self.unary(
            "/command_server.CommandServer/Cancel",
            request,
            Some(deadline),
        )
        .await
    }

    /// `rpc UpdateTerminalSize(TerminalSizeRequest) returns (TerminalSizeResponse)`.
    pub async fn update_terminal_size(
        &mut self,
        request: TerminalSizeRequest,
        deadline: Duration,
    ) -> Result<TerminalSizeResponse, GrpcStatus> {
        self.unary(
            "/command_server.CommandServer/UpdateTerminalSize",
            request,
            Some(deadline),
        )
        .await
    }

    /// `rpc Run(RunRequest) returns (stream RunResponse)`: starts the call (no deadline, as the
    /// C++ sets none) and returns the response stream once the server has answered with its
    /// headers.
    pub async fn run(&mut self, request: RunRequest) -> Result<RunReader, GrpcStatus> {
        let call = async {
            self.ready().await?;
            let codec = tonic_prost::ProstCodec::<RunRequest, RunResponse>::default();
            self.inner
                .server_streaming(
                    tonic::Request::new(request),
                    PathAndQuery::from_static("/command_server.CommandServer/Run"),
                    codec,
                )
                .await
        };
        match call.await {
            Ok(response) => Ok(RunReader {
                stream: Some(response.into_inner()),
                status: None,
            }),
            Err(s) => Err(GrpcStatus::from_tonic(&s)),
        }
    }
}

/// `grpc::ClientReader<RunResponse>`: `Read` returns messages until the stream ends; `Finish`
/// then gives the call's status.
pub struct RunReader {
    stream: Option<tonic::codec::Streaming<RunResponse>>,
    status: Option<GrpcStatus>,
}

impl RunReader {
    /// A reader for a call that failed before any message (`Read` returns false at once and
    /// `Finish` returns `status`).
    pub fn failed(status: GrpcStatus) -> Self {
        RunReader {
            stream: None,
            status: Some(status),
        }
    }

    /// `reader->Read(&response)`: the next message, or `None` at the end of the stream (or on
    /// an error, which `finish` then reports).
    pub async fn read(&mut self) -> Option<RunResponse> {
        let stream = self.stream.as_mut()?;
        match stream.message().await {
            Ok(Some(response)) => Some(response),
            Ok(None) => {
                self.stream = None;
                self.status = Some(GrpcStatus::ok());
                None
            }
            Err(s) => {
                self.stream = None;
                self.status = Some(GrpcStatus::from_tonic(&s));
                None
            }
        }
    }

    /// `reader->Finish()`: the status of the call. Call after `read` returned `None`.
    pub fn finish(&mut self) -> GrpcStatus {
        self.stream = None;
        self.status.take().unwrap_or_else(GrpcStatus::ok)
    }
}
