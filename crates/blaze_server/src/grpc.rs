//! The gRPC layer: the generated `command_server.CommandServer` stubs and a stand-in for
//! `io.grpc.Server` (start, port, `shutdown`, `shutdownNow`, `awaitTermination`) on tonic.
//!
//! PORT: Java serves through grpc-java's Netty server (`NettyServerBuilder`); Blaze serves the
//! same service over plain HTTP/2 (prior knowledge, as the C++ client connects) with tonic on a
//! tokio runtime owned by [`Server`].

use std::io;
use std::marker::PhantomData;
use std::net::SocketAddr;
use std::sync::Arc;

use parking_lot::{Condvar, Mutex};
use tokio::sync::watch;
use tonic::codec::{BufferSettings, Codec};

/// Stubs of `command_server.CommandServer` (`command_server_server` and
/// `command_server_client`). Java: the generated `CommandServerGrpc` class.
#[allow(clippy::all, clippy::pedantic, missing_docs, unused_qualifications)]
pub mod stubs {
    include!(concat!(env!("OUT_DIR"), "/command_server.CommandServer.rs"));
}

/// The codec of the `CommandServer` stubs: prost messages, with tonic's encoder handing each
/// message to HTTP/2 on its own.
///
/// PORT: tonic's default encoder gathers up to 32 KiB of ready messages into one HTTP/2 data
/// chunk before it yields; grpc-java writes (and flushes) each message by itself. Yielding per
/// message keeps the amount a blocked client leaves queued on the server at about one message
/// beyond the HTTP/2 window, as `BlockingStreamObserver` (`isReady`) expects.
#[derive(Debug, Clone)]
pub struct CommandServerCodec<T, U>(PhantomData<(T, U)>);

impl<T, U> Default for CommandServerCodec<T, U> {
    fn default() -> Self {
        CommandServerCodec(PhantomData)
    }
}

/// tonic's `DEFAULT_CODEC_BUFFER_SIZE`.
const CODEC_BUFFER_SIZE: usize = 8 * 1024;

impl<T, U> Codec for CommandServerCodec<T, U>
where
    T: prost::Message + Send + 'static,
    U: prost::Message + Default + Send + 'static,
{
    type Encode = T;
    type Decode = U;
    type Encoder = tonic_prost::ProstEncoder<T>;
    type Decoder = tonic_prost::ProstDecoder<U>;

    fn encoder(&mut self) -> Self::Encoder {
        // A yield threshold of 0: every encoded message is yielded at once.
        tonic_prost::ProstEncoder::new(BufferSettings::new(CODEC_BUFFER_SIZE, 0))
    }

    fn decoder(&mut self) -> Self::Decoder {
        tonic_prost::ProstDecoder::new(BufferSettings::default())
    }
}

/// The `shutdown()` of an `io.grpc.Server`: begin an orderly shutdown in which in-flight calls
/// finish and new calls are rejected.
///
/// PORT: a trait so that `ServerWatcherRunnableTest` can use a mock server, as Java does.
pub trait ServerShutdown: Send + Sync {
    fn shutdown(&self);
}

/// A handle that shuts a [`Server`] down.
#[derive(Clone)]
pub struct ServerHandle {
    shutdown: watch::Sender<bool>,
    /// The runtime that runs the server and every connection; `shutdownNow` drops it.
    runtime: Arc<Mutex<Option<tokio::runtime::Runtime>>>,
    terminated: Arc<(Mutex<bool>, Condvar)>,
}

impl ServerShutdown for ServerHandle {
    fn shutdown(&self) {
        self.shutdown.send_replace(true);
    }
}

impl ServerHandle {
    /// `shutdownNow()`: a forceful shutdown in which in-flight calls are cancelled. Every
    /// connection is dropped, so clients see their streams fail and the servers' response
    /// streams are dropped (their `BlockingStreamObserver`s become cancelled).
    pub fn shutdown_now(&self) {
        self.shutdown.send_replace(true);
        if let Some(runtime) = self.runtime.lock().take() {
            // Safe from any thread, including one of the runtime's own.
            runtime.shutdown_background();
        }
        self.set_terminated();
    }

    fn set_terminated(&self) {
        let (lock, cond) = &*self.terminated;
        *lock.lock() = true;
        cond.notify_all();
    }

    /// `awaitTermination()`: waits until the server has shut down and every connection is
    /// closed.
    pub fn await_termination(&self) {
        let (lock, cond) = &*self.terminated;
        let mut terminated = lock.lock();
        while !*terminated {
            cond.wait(&mut terminated);
        }
    }
}

/// A started gRPC server: the stand-in for `io.grpc.Server`.
pub struct Server {
    address: SocketAddr,
    handle: ServerHandle,
}

impl Server {
    /// Serves `service` on the already bound `listener`.
    ///
    /// Java's `NettyServerBuilder.forAddress(address).addService(this).directExecutor().build()
    /// .start()` binds in `start`; here the caller binds, so that it can fall back from IPv6 to
    /// IPv4 as Java does.
    pub fn start<T: stubs::command_server_server::CommandServer>(
        listener: std::net::TcpListener,
        service: T,
    ) -> io::Result<Server> {
        let service = stubs::command_server_server::CommandServerServer::new(service);
        let address = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("grpc-nio-worker")
            .enable_all()
            .build()?;
        let listener = {
            let _guard = runtime.enter();
            tokio::net::TcpListener::from_std(listener)?
        };
        let (tx, mut rx) = watch::channel(false);
        let handle = ServerHandle {
            shutdown: tx,
            runtime: Arc::new(Mutex::new(None)),
            terminated: Arc::new((Mutex::new(false), Condvar::new())),
        };
        let done = handle.clone();
        runtime.spawn(async move {
            let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
            // On shutdown, stop accepting and wait for the open connections to finish their
            // calls (HTTP/2 GOAWAY).
            let _ = tonic::transport::Server::builder()
                .add_service(service)
                .serve_with_incoming_shutdown(incoming, async move {
                    let _ = rx.wait_for(|shutdown| *shutdown).await;
                })
                .await;
            done.set_terminated();
        });
        *handle.runtime.lock() = Some(runtime);
        Ok(Server { address, handle })
    }

    /// `getPort()`.
    pub fn get_port(&self) -> u16 {
        self.address.port()
    }

    /// The bound address.
    pub fn get_address(&self) -> SocketAddr {
        self.address
    }

    /// A handle for `shutdown()` / `shutdownNow()` / `awaitTermination()`.
    pub fn handle(&self) -> ServerHandle {
        self.handle.clone()
    }

    /// `shutdown()`.
    pub fn shutdown(&self) {
        self.handle.shutdown();
    }

    /// `shutdownNow()`.
    pub fn shutdown_now(&self) {
        self.handle.shutdown_now();
    }

    /// `awaitTermination()`.
    pub fn await_termination(&self) {
        self.handle.await_termination();
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.handle.shutdown_now();
    }
}
