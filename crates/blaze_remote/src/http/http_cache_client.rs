//! Port of `lib/remote/http/HttpCacheClient.java` and of the request/response handling of its
//! Netty handlers (`AbstractHttpHandler`, `HttpDownloadHandler`, `HttpUploadHandler`,
//! `IdleTimeoutHandler`).
//!
//! Blobs (Binary large objects) are uploaded using the `PUT` method. Action cache blobs are
//! stored under the path `/ac/base16-key`. CAS (Content Addressable Storage) blobs are stored
//! under the path `/cas/base16-key`. Valid status codes for a successful upload are 200 (OK),
//! 201 (CREATED), 202 (ACCEPTED) and 204 (NO CONTENT). It's recommended to return 200 (OK) on
//! success. The other status codes are supported to be compatibility with the nginx webdav module
//! and may be removed in the future.
//!
//! Blobs are downloaded using the `GET` method at the paths they were stored at. A status code of
//! 200 should be followed by the content of the blob. The status codes 404 (NOT FOUND) and 204
//! (NO CONTENT) indicate that no cache entry exists. It's recommended to return 404 (NOT FOUND)
//! as the 204 (NO CONTENT) status code is only supported for compatibility with the nginx webdav
//! module.
//!
//! TLS is supported and enabled automatically when using HTTPS as the URI scheme.
//!
//! Uploads do not use `Expect: 100-CONTINUE` headers, as this would incur an additional roundtrip
//! for every upload and with little practical value as we would expect most uploads to be
//! accepted.
//!
//! PORT: Java runs each request through a pipeline of Netty handlers on an event loop, over a
//! `FixedChannelPool` (`--remote_max_connections` > 0) or a `SimpleChannelPool`. Here a request
//! runs on a blocking connection taken from [`ConnectionPool`] (same limit, connections kept
//! alive and reused last-in first-out as Netty's pool does), on the calling task's thread
//! (`tokio::task::block_in_place`); the response is read with the downloader's HTTP stack
//! (`blaze_repo_fetch::downloader::socket_transport`). The idle timeout (`IdleTimeoutHandler`:
//! no read or write for `--remote_timeout`) is the socket's read and write timeout.
//!
//! Where the Netty plumbing went:
//! - `acquireUploadChannel` / `acquireDownloadChannel`: [`ConnectionPool::acquire`] then
//!   `connect`; `releaseUploadChannel` / `releaseDownloadChannel`: [`ConnectionPool::release`]
//!   (a connection is kept only after a complete keep-alive response).
//! - `HttpUploadHandler.buildRequest` / `HttpDownloadHandler.buildRequest`: the request heads of
//!   `upload` and `download`; `HttpUploadHandler.buildBody`: the body loop of `exchange_upload`.
//! - `HttpDownloadHandler.succeedAndReset`, `failAndClose`, `failAndReset` and
//!   `HttpUploadHandler.failAndClose`: the outcome of `exchange_download` / `exchange_upload`
//!   with the connection kept or dropped; their `exceptionCaught` (the timeout exceptions) is the
//!   error mapping there.
//! - `uploadAfterCredentialRefresh`: the second `upload` in `upload_async`.
//! - `DownloadCommand` and `UploadCommand` are the parameters of `download` / `upload`.
//
// Java: com/google/devtools/build/lib/remote/http/HttpCacheClient.java:304 HttpCacheClient.acquireUploadChannel
// Java: com/google/devtools/build/lib/remote/http/HttpCacheClient.java:359 HttpCacheClient.releaseUploadChannel
// Java: com/google/devtools/build/lib/remote/http/HttpCacheClient.java:379 HttpCacheClient.acquireDownloadChannel
// Java: com/google/devtools/build/lib/remote/http/HttpCacheClient.java:427 HttpCacheClient.releaseDownloadChannel
// Java: com/google/devtools/build/lib/remote/http/HttpCacheClient.java:695 HttpCacheClient.uploadAfterCredentialRefresh
// Java: com/google/devtools/build/lib/remote/http/HttpUploadHandler.java:124 HttpUploadHandler.exceptionCaught
// Java: com/google/devtools/build/lib/remote/http/HttpUploadHandler.java:136 HttpUploadHandler.buildRequest
// Java: com/google/devtools/build/lib/remote/http/HttpUploadHandler.java:145 HttpUploadHandler.buildBody
// Java: com/google/devtools/build/lib/remote/http/HttpUploadHandler.java:149 HttpUploadHandler.failAndClose
// Java: com/google/devtools/build/lib/remote/http/HttpDownloadHandler.java:175 HttpDownloadHandler.exceptionCaught
// Java: com/google/devtools/build/lib/remote/http/HttpDownloadHandler.java:184 HttpDownloadHandler.buildRequest
// Java: com/google/devtools/build/lib/remote/http/HttpDownloadHandler.java:194 HttpDownloadHandler.succeedAndReset
// Java: com/google/devtools/build/lib/remote/http/HttpDownloadHandler.java:208 HttpDownloadHandler.failAndClose
// Java: com/google/devtools/build/lib/remote/http/HttpDownloadHandler.java:219 HttpDownloadHandler.failAndReset
// Java: com/google/devtools/build/lib/remote/http/AbstractHttpHandler.java:55 AbstractHttpHandler.failAndResetUserPromise
// Java: com/google/devtools/build/lib/remote/http/AbstractHttpHandler.java:114 AbstractHttpHandler.exceptionCaught
// Java: com/google/devtools/build/lib/remote/http/IdleTimeoutHandler.java:38 IdleTimeoutHandler.channelIdle
// PORT: skipped, Netty channel plumbing with no counterpart on a blocking connection (each forwards the event and fails the pending promise with ClosedChannelException, which the outcome of the exchange is here): AbstractHttpHandler.bind, AbstractHttpHandler.connect, AbstractHttpHandler.disconnect, AbstractHttpHandler.close, AbstractHttpHandler.deregister, AbstractHttpHandler.read, AbstractHttpHandler.flush, AbstractHttpHandler.channelInactive, AbstractHttpHandler.handlerRemoved, AbstractHttpHandler.channelUnregistered, HttpCacheClient.isChannelPipelineEmpty
// PORT: skipped, handler and command objects: the client's fields and the method parameters hold their state: AbstractHttpHandler.<init>, HttpUploadHandler.<init>, HttpDownloadHandler.<init>, IdleTimeoutHandler.<init>, DownloadCommand.<init>, DownloadCommand.uri, DownloadCommand.casDownload, DownloadCommand.digest, DownloadCommand.out, DownloadCommand.offset, UploadCommand.<init>, UploadCommand.uri, UploadCommand.casUpload, UploadCommand.hash, UploadCommand.data, UploadCommand.contentLength
// PORT: skipped, the response is kept as its status and headers on the exception (`HttpException.status`, `headers`): HttpException.response

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use blaze_repo_fetch::downloader::socket_transport::{
    BodyFraming, BodyStream, ResponseHead, body_framing, read_response_head,
};
use blaze_repo_fetch::downloader::streams::InputStream;
use blaze_repo_fetch::errors::{IoException, IoExceptionKind};
use blaze_repo_fetch::uri::JavaUri;
use indexmap::IndexSet;
use parking_lot::{Condvar, Mutex};
use prost::Message;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig, ClientConnection, StreamOwned};

use super::{
    ClosedChannelException, DownloadTimeoutException, HttpException, UploadTimeoutException,
};
use crate::authandtls::Credentials;
use crate::common::cache_not_found_exception::CacheNotFoundException;
use crate::common::errors;
use crate::common::remote_action_execution_context::RemoteActionExecutionContext;
use crate::common::remote_cache_client::{
    ActionKey, Blob, MissingDigestsFinder, RemoteCacheClient,
};
use crate::reapi::{
    ActionCacheUpdateCapabilities, ActionResult, CacheCapabilities, Digest, ServerCapabilities,
    symlink_absolute_path_strategy,
};
use crate::remote_retrier::RemoteRetrier;
use crate::util::async_task_cache::AsyncTaskCacheNoResult;
use crate::util::digest_util::DigestUtil;
use crate::util::utils;

// Java: com/google/devtools/build/lib/remote/http/HttpCacheClient.java:127 HttpCacheClient.AC_PREFIX
pub const AC_PREFIX: &str = "ac/";
// Java: com/google/devtools/build/lib/remote/http/HttpCacheClient.java:128 HttpCacheClient.CAS_PREFIX
pub const CAS_PREFIX: &str = "cas/";

/// `HttpDownloadHandler.ACCEPT_ENCODING`: gzip, deflate, snappy and zstd (`Zstd.isAvailable()`
/// in the oracle: its request heads carry `accept-encoding: gzip,deflate,snappy,zstd`).
// Java: com/google/devtools/build/lib/remote/http/HttpDownloadHandler.java:243 HttpDownloadHandler.getAcceptEncoding
const ACCEPT_ENCODING: &str = "gzip,deflate,snappy,zstd";

/// `HttpObjectAggregator(10 * 1024)` of the upload pipeline: the most of an upload response's
/// body that is read.
const MAX_UPLOAD_RESPONSE_CONTENT: usize = 10 * 1024;

/// Where the connections go: the URI's host and port, or a Unix domain socket
/// (`--remote_proxy=unix:<path>`).
#[derive(Debug, Clone)]
pub enum CacheAddress {
    Inet { host: String, port: u16 },
    DomainSocket(PathBuf),
}

/// A connection of the pool.
enum Stream {
    Tcp(TcpStream),
    Unix(UnixStream),
    TlsTcp(Box<StreamOwned<ClientConnection, TcpStream>>),
    TlsUnix(Box<StreamOwned<ClientConnection, UnixStream>>),
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Stream::Tcp(s) => s.read(buf),
            Stream::Unix(s) => s.read(buf),
            Stream::TlsTcp(s) => s.read(buf),
            Stream::TlsUnix(s) => s.read(buf),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Stream::Tcp(s) => s.write(buf),
            Stream::Unix(s) => s.write(buf),
            Stream::TlsTcp(s) => s.write(buf),
            Stream::TlsUnix(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Stream::Tcp(s) => s.flush(),
            Stream::Unix(s) => s.flush(),
            Stream::TlsTcp(s) => s.flush(),
            Stream::TlsUnix(s) => s.flush(),
        }
    }
}

type Connection = BufReader<Stream>;

/// Whether an idle connection is still open (`Channel.isActive()`): nothing buffered, and the
/// socket neither at end of stream nor in error.
fn is_open(connection: &Connection) -> bool {
    if !connection.buffer().is_empty() {
        // Bytes nobody asked for: not a connection to reuse.
        return false;
    }
    fn peek_open(
        set_nonblocking: impl Fn(bool) -> io::Result<()>,
        peek: impl Fn() -> io::Result<usize>,
    ) -> bool {
        if set_nonblocking(true).is_err() {
            return false;
        }
        let open = match peek() {
            Ok(_) => false,
            Err(e) => e.kind() == io::ErrorKind::WouldBlock,
        };
        set_nonblocking(false).is_ok() && open
    }
    let mut byte = [0u8; 1];
    let byte = std::cell::RefCell::new(&mut byte);
    match connection.get_ref() {
        Stream::Tcp(s) => peek_open(
            |b| s.set_nonblocking(b),
            || s.peek(&mut **byte.borrow_mut()),
        ),
        Stream::TlsTcp(s) => {
            let s = s.get_ref();
            peek_open(
                |b| s.set_nonblocking(b),
                || s.peek(&mut **byte.borrow_mut()),
            )
        }
        Stream::Unix(s) => peek_open(|b| s.set_nonblocking(b), || unix_peek(s)),
        Stream::TlsUnix(s) => {
            let s = s.get_ref();
            peek_open(|b| s.set_nonblocking(b), || unix_peek(s))
        }
    }
}

/// `recv(MSG_PEEK)` of one byte on a Unix socket.
fn unix_peek(s: &UnixStream) -> io::Result<usize> {
    use std::os::fd::AsRawFd;
    let mut byte = 0u8;
    // SAFETY: one byte into a valid buffer on an open descriptor.
    let n = unsafe {
        libc::recv(
            s.as_raw_fd(),
            (&mut byte as *mut u8).cast(),
            1,
            libc::MSG_PEEK,
        )
    };
    if n < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(n as usize)
    }
}

/// The channel pool: at most `max` connections in use when `max` is set (`FixedChannelPool`;
/// further acquisitions wait), idle connections kept for reuse, the most recently released
/// first (`SimpleChannelPool`'s `lastRecentUsed`).
struct ConnectionPool {
    state: Mutex<PoolState>,
    released: Condvar,
    max: Option<usize>,
}

struct PoolState {
    idle: Vec<Connection>,
    in_use: usize,
    closed: bool,
}

impl ConnectionPool {
    fn new(max: Option<usize>) -> ConnectionPool {
        ConnectionPool {
            state: Mutex::new(PoolState {
                idle: Vec::new(),
                in_use: 0,
                closed: false,
            }),
            released: Condvar::new(),
            max,
        }
    }

    /// Takes a slot, and an idle connection if there is one.
    fn acquire(&self) -> io::Result<Option<Connection>> {
        let mut state = self.state.lock();
        loop {
            if state.closed {
                // FixedChannelPool.acquire after close: IllegalStateException("FixedChannelPool
                // was closed").
                return Err(errors::io_exception("FixedChannelPool was closed"));
            }
            if self.max.is_none_or(|max| state.in_use < max) {
                state.in_use += 1;
                // ChannelHealthChecker.ACTIVE: a connection the server closed is not reused.
                while let Some(connection) = state.idle.pop() {
                    if is_open(&connection) {
                        return Ok(Some(connection));
                    }
                }
                return Ok(None);
            }
            self.released.wait(&mut state);
        }
    }

    /// Gives the slot back, with the connection when it can carry another request.
    fn release(&self, connection: Option<Connection>) {
        let mut state = self.state.lock();
        state.in_use -= 1;
        if let Some(connection) = connection
            && !state.closed
        {
            state.idle.push(connection);
        }
        self.released.notify_one();
    }

    /// `channelPool.close()`: closes the idle connections.
    fn close(&self) {
        let mut state = self.state.lock();
        state.closed = true;
        state.idle.clear();
        self.released.notify_all();
    }
}

/// Implementation of [`RemoteCacheClient`] that can talk to a HTTP/1.1 backend.
// Java: com/google/devtools/build/lib/remote/http/HttpCacheClient.java:124 HttpCacheClient
pub struct HttpCacheClient {
    uri: JavaUri,
    timeout_seconds: i32,
    extra_http_headers: Vec<(String, String)>,
    use_tls: bool,
    verify_downloads: bool,
    digest_util: DigestUtil,
    retrier: Arc<RemoteRetrier>,
    address: CacheAddress,
    tls: Option<Arc<ClientConfig>>,
    pool: ConnectionPool,
    is_closed: AtomicBool,
    /// `creds` (guarded by `credentialsLock`).
    creds: Option<Arc<dyn Credentials>>,
    /// `lastRefreshTime`, in milliseconds since the epoch (guarded by `credentialsLock`).
    last_refresh_time: Mutex<i64>,
    /// `AbstractHttpHandler.USER_AGENT_VALUE`: `"bazel/" + BlazeVersionInfo.getVersion()`.
    user_agent: String,
    cas_upload_cache: AsyncTaskCacheNoResult<Digest>,
}

impl HttpCacheClient {
    /// `HttpCacheClient.create(uri, ...)` (an internet address) and
    /// `HttpCacheClient.create(domainSocketAddress, uri, ...)` (`domain_socket`).
    ///
    /// `tls` is the configuration `createSSLContext` stands for
    /// (`google_auth_utils::http_cache_tls_config`), required for an `https` URI.
    /// `tool_version` is `BlazeVersionInfo.instance().getVersion()`.
    // Java: com/google/devtools/build/lib/remote/http/HttpCacheClient.java:154 HttpCacheClient.create
    // Java: com/google/devtools/build/lib/remote/http/HttpCacheClient.java:180 HttpCacheClient.create
    // Java: com/google/devtools/build/lib/remote/http/HttpCacheClient.java:226 HttpCacheClient.<init>
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        domain_socket: Option<PathBuf>,
        uri: JavaUri,
        timeout_seconds: i32,
        remote_max_connections: i32,
        verify_downloads: bool,
        extra_http_headers: Vec<(String, String)>,
        digest_util: DigestUtil,
        retrier: Arc<RemoteRetrier>,
        creds: Option<Arc<dyn Credentials>>,
        tls: Option<Arc<ClientConfig>>,
        tool_version: &str,
    ) -> io::Result<HttpCacheClient> {
        let use_tls = uri.get_scheme() == Some("https");
        let mut uri = uri;
        if uri.get_port() == -1 {
            let port = if use_tls { 443 } else { 80 };
            uri = JavaUri::from_components(
                uri.get_scheme(),
                uri.get_user_info().as_deref(),
                uri.get_host(),
                port,
                uri.get_path().as_deref(),
                uri.get_query().as_deref(),
                uri.get_fragment().as_deref(),
            )
            .map_err(|e| errors::io_exception(e.to_string()))?;
        }
        let address = match domain_socket {
            Some(path) => CacheAddress::DomainSocket(path),
            None => CacheAddress::Inet {
                // `new InetSocketAddress(uri.getHost(), uri.getPort())`
                host: uri.get_host().unwrap_or_default().to_string(),
                port: uri.get_port() as u16,
            },
        };
        if use_tls && tls.is_none() {
            panic!("IllegalStateException: an https remote cache needs its TLS configuration");
        }
        let max = (remote_max_connections > 0).then_some(remote_max_connections as usize);
        Ok(HttpCacheClient {
            uri,
            timeout_seconds,
            extra_http_headers,
            use_tls,
            verify_downloads,
            digest_util,
            retrier,
            address,
            tls: if use_tls { tls } else { None },
            pool: ConnectionPool::new(max),
            is_closed: AtomicBool::new(false),
            creds,
            last_refresh_time: Mutex::new(0),
            user_agent: format!("bazel/{tool_version}"),
            cas_upload_cache: AsyncTaskCacheNoResult::create(),
        })
    }

    /// The idle timeout (`IdleTimeoutHandler(timeoutSeconds, ...)`; 0 disables it).
    fn idle_timeout(&self) -> Option<Duration> {
        (self.timeout_seconds > 0).then(|| Duration::from_secs(self.timeout_seconds as u64))
    }

    /// Opens a connection: `Bootstrap.connect` with `CONNECT_TIMEOUT_MILLIS = 1000 *
    /// timeoutSeconds`, then the TLS handshake of the pool's `ssl-handler`.
    fn connect(&self) -> io::Result<Connection> {
        let connect_timeout = self.idle_timeout();
        let host = self.uri.get_host().unwrap_or_default().to_string();
        let port = self.uri.get_port();
        let stream = match &self.address {
            CacheAddress::Inet { host, port } => {
                // `new InetSocketAddress(host, port)` resolves the host once
                // (`InetAddress.getByName`): its first address, IPv4 addresses before IPv6 ones
                // (the JDK's default `java.net.preferIPv6Addresses=false`).
                let addresses: Vec<SocketAddr> = (host.as_str(), *port)
                    .to_socket_addrs()
                    .map_err(|e| unknown_host(host, &e))?
                    .collect();
                let address = addresses
                    .iter()
                    .find(|a| a.is_ipv4())
                    .or_else(|| addresses.first())
                    .copied()
                    .ok_or_else(|| {
                        unknown_host(host, &io::Error::other("Name or service not known"))
                    })?;
                let tcp = match connect_timeout {
                    Some(timeout) => TcpStream::connect_timeout(&address, timeout),
                    None => TcpStream::connect(address),
                }
                .map_err(|e| connect_exception(&e, host, &address))?;
                tcp.set_read_timeout(self.idle_timeout())?;
                tcp.set_write_timeout(self.idle_timeout())?;
                let _ = tcp.set_nodelay(true);
                match &self.tls {
                    Some(tls) => Stream::TlsTcp(Box::new(StreamOwned::new(
                        client_connection(tls, &host_for_tls(&self.uri))?,
                        tcp,
                    ))),
                    None => Stream::Tcp(tcp),
                }
            }
            CacheAddress::DomainSocket(path) => {
                let unix = UnixStream::connect(path).map_err(|e| {
                    errors::io_exception(format!(
                        "connect(..) failed: {}: {}",
                        java_io_message(&e),
                        path.display()
                    ))
                })?;
                unix.set_read_timeout(self.idle_timeout())?;
                unix.set_write_timeout(self.idle_timeout())?;
                match &self.tls {
                    Some(tls) => Stream::TlsUnix(Box::new(StreamOwned::new(
                        client_connection(tls, &host_for_tls(&self.uri))?,
                        unix,
                    ))),
                    None => Stream::Unix(unix),
                }
            }
        };
        let _ = (host, port);
        Ok(BufReader::new(stream))
    }

    /// `downloadBlob` / `downloadActionResult`'s `get(digest, out, casBytesDownloaded)`: one
    /// download attempt with its refresh of expired credentials.
    // Java: com/google/devtools/build/lib/remote/http/HttpCacheClient.java:479 HttpCacheClient.get
    fn get(
        &self,
        digest: &Digest,
        out: &mut dyn Write,
        cas_bytes_downloaded: Option<&AtomicI64>,
    ) -> io::Result<()> {
        let data_written = AtomicBool::new(false);
        let offset = cas_bytes_downloaded.map_or(0, |c| c.load(Ordering::SeqCst));
        let mut wrapped_out = CountingOut {
            out,
            data_written: &data_written,
            cas_bytes_downloaded,
        };
        let cas_download = cas_bytes_downloaded.is_some();
        match self.download(digest, cas_download, &mut wrapped_out, offset) {
            Ok(()) => Ok(()),
            Err(cause) => {
                let Some(http_exception) = errors::downcast_ref::<HttpException>(&cause) else {
                    return Err(cause);
                };
                if !data_written.load(Ordering::SeqCst) && self.auth_token_expired(http_exception) {
                    // The error is due to an auth token having expired. Let's try again.
                    match self.refresh_credentials() {
                        Ok(()) => {
                            return self.get_after_credential_refresh(
                                digest,
                                cas_download,
                                &mut wrapped_out,
                                offset,
                            );
                        }
                        // cause.addSuppressed(e): the cause is what fails.
                        Err(_suppressed) => return Err(cause),
                    }
                } else if cache_miss(http_exception.status) {
                    return Err(CacheNotFoundException::new(digest.clone()).into());
                }
                Err(cause)
            }
        }
    }

    // Java: com/google/devtools/build/lib/remote/http/HttpCacheClient.java:570 HttpCacheClient.getAfterCredentialRefresh
    fn get_after_credential_refresh(
        &self,
        digest: &Digest,
        cas_download: bool,
        out: &mut dyn Write,
        offset: i64,
    ) -> io::Result<()> {
        match self.download(digest, cas_download, out, offset) {
            Ok(()) => Ok(()),
            Err(cause) => {
                if let Some(http_exception) = errors::downcast_ref::<HttpException>(&cause)
                    && cache_miss(http_exception.status)
                {
                    return Err(CacheNotFoundException::new(digest.clone()).into());
                }
                Err(cause)
            }
        }
    }

    /// One `DownloadCommand` on a pooled connection: `acquireDownloadChannel`,
    /// `HttpDownloadHandler.write` and `channelRead0`, `releaseDownloadChannel`.
    // Java: com/google/devtools/build/lib/remote/http/HttpDownloadHandler.java:149 HttpDownloadHandler.write
    // Java: com/google/devtools/build/lib/remote/http/HttpDownloadHandler.java:70 HttpDownloadHandler.channelRead0
    fn download(
        &self,
        digest: &Digest,
        cas_download: bool,
        out: &mut dyn Write,
        offset: i64,
    ) -> io::Result<()> {
        let path = construct_path(&self.uri, &digest.hash, cas_download);
        let mut headers: Vec<(String, String)> = Vec::new();
        // buildRequest(path, constructHost(uri))
        set_header(&mut headers, "host", &construct_host(&self.uri));
        set_header(&mut headers, "connection", "keep-alive");
        set_header(&mut headers, "accept", "*/*");
        set_header(&mut headers, "accept-encoding", ACCEPT_ENCODING);
        self.add_credential_headers(&mut headers)?;
        self.add_extra_remote_headers(&mut headers);
        self.add_user_agent_header(&mut headers);
        let request = request_head("GET", &path, &headers);

        let connection = match self.pool.acquire()? {
            Some(connection) => connection,
            None => match self.connect() {
                Ok(connection) => connection,
                Err(e) => {
                    self.pool.release(None);
                    return Err(e);
                }
            },
        };
        let mut state = DownloadState {
            path: path.clone(),
            bytes_received: 0,
            content_length: -1,
        };
        let (kept, result) = Self::exchange_download(connection, &request, out, offset, &mut state);
        self.pool.release(kept);
        result
    }

    /// Writes the request and reads the response; gives the connection back when it can carry
    /// another request (`reset`: kept alive; `failAndClose`: closed).
    fn exchange_download(
        mut connection: Connection,
        request: &[u8],
        out: &mut dyn Write,
        offset: i64,
        state: &mut DownloadState,
    ) -> (Option<Connection>, io::Result<()>) {
        let timeout = |state: &DownloadState| -> io::Error {
            DownloadTimeoutException::new(&state.path, state.bytes_received, state.content_length)
                .into()
        };
        let close = |e: io::Error| -> (Option<Connection>, io::Result<()>) { (None, Err(e)) };
        if let Err(e) = write_all_flush(connection.get_mut(), request) {
            return close(if is_timeout(&e) {
                timeout(state)
            } else {
                closed_or(e)
            });
        }
        let head = match read_response_head(&mut connection) {
            Ok(head) => head,
            Err(e) => {
                return close(match head_failure(&e) {
                    HeadFailure::Timeout => timeout(state),
                    HeadFailure::Closed => ClosedChannelException.into(),
                    HeadFailure::Parse => {
                        errors::io_exception("Failed to parse the HTTP response.")
                    }
                    HeadFailure::Other => io_from_repo_fetch(&e),
                });
            }
        };
        let version = head.status_line.split(' ').next().unwrap_or("");
        if version != "HTTP/1.1" {
            return close(
                http_exception(
                    &head,
                    format!("HTTP version 1.1 is required, was: {version}"),
                )
                .into(),
            );
        }
        let content_length_set = head.header("content-length").is_some();
        let chunked = head
            .header("transfer-encoding")
            .is_some_and(|te| te.eq_ignore_ascii_case("chunked"));
        if !content_length_set && !chunked {
            return close(
                http_exception(
                    &head,
                    "Missing 'Content-Length' or 'Transfer-Encoding: chunked' header".to_string(),
                )
                .into(),
            );
        }
        // HttpContentDecompressor: a decoded response has no Content-Length any more.
        let decoder = match content_decoder(&head) {
            Ok(decoder) => decoder,
            Err(e) => return close(e),
        };
        if content_length_set && decoder.is_none() {
            state.content_length = head
                .header("content-length")
                .and_then(|l| l.trim().parse::<i64>().ok())
                .unwrap_or(-1);
        }
        let download_succeeded = head.code == 200;
        let keep_alive = is_keep_alive(&head);

        let framing = body_framing(&head);
        // The body is read through the connection, which is taken back afterwards.
        let mut body = BodyStream::new(connection, framing);
        let mut raw = RawBody {
            body: &mut body,
            fixed_remaining: match framing {
                BodyFraming::Fixed(n) => Some(n),
                _ => None,
            },
        };
        let mut decoded: Box<dyn Read + '_> = match decoder {
            None => Box::new(&mut raw),
            Some(ContentDecoder::Gzip) => Box::new(flate2::read::MultiGzDecoder::new(&mut raw)),
            Some(ContentDecoder::Deflate) => Box::new(zlib_or_none(&mut raw)),
            Some(ContentDecoder::Snappy) => {
                Box::new(super::snappy::SnappyFrameDecoder::new(&mut raw))
            }
            Some(ContentDecoder::Zstd) => match zstd::stream::read::Decoder::new(&mut raw) {
                Ok(decoder) => Box::new(decoder),
                Err(e) => return close(e),
            },
        };

        let mut skip_bytes = offset.max(0) as u64;
        let mut error_body: Vec<u8> = Vec::new();
        let mut buf = vec![0u8; 64 * 1024];
        let read_result: io::Result<()> = loop {
            let n = match decoded.read(&mut buf) {
                Ok(0) => break Ok(()),
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    // Netty's `DecompressionException` (of the snappy decoder) is a
                    // `RuntimeException`: it fails the download's future unchecked, and Bazel
                    // crashes with it ("FATAL: bazel crashed due to an internal error"; the
                    // oracle on a reserved unskippable snappy chunk, review round 8 S3).
                    if let Some(java) = errors::downcast_ref::<errors::JavaException>(&e)
                        && java.class_name
                            == "io.netty.handler.codec.compression.DecompressionException"
                    {
                        panic!("{}: {}", java.class_name, java.message);
                    }
                    break Err(e);
                }
            };
            let mut chunk = &buf[..n];
            if skip_bytes > 0 {
                let skip_now = skip_bytes.min(chunk.len() as u64) as usize;
                chunk = &chunk[skip_now..];
                skip_bytes -= skip_now as u64;
            }
            if download_succeeded {
                if let Err(e) = out.write_all(chunk) {
                    break Err(e);
                }
            } else {
                error_body.extend_from_slice(chunk);
            }
            state.bytes_received += chunk.len() as i64;
        };
        drop(decoded);
        if let Err(e) = read_result {
            return close(if is_timeout(&e) { timeout(state) } else { e });
        }
        let Some(connection) = body.into_reader() else {
            return close(ClosedChannelException.into());
        };
        let kept = keep_alive.then_some(connection);
        if download_succeeded {
            (kept, Ok(()))
        } else {
            // response.status() + "\n" + the body in the response's charset.
            let error_msg = format!(
                "{} {}\n{}",
                head.code,
                head.message,
                decode_charset(&head, &error_body)
            );
            (kept, Err(http_exception(&head, error_msg).into()))
        }
    }

    /// One `UploadCommand` (`uploadAsync` with its refresh of expired credentials).
    // Java: com/google/devtools/build/lib/remote/http/HttpCacheClient.java:634 HttpCacheClient.uploadAsync
    fn upload_async(
        &self,
        key: &str,
        length: i64,
        blob: &dyn Blob,
        cas_upload: bool,
    ) -> io::Result<()> {
        match self.upload(key, length, blob, cas_upload) {
            Ok(()) => Ok(()),
            Err(cause) => {
                let Some(http_exception) = errors::downcast_ref::<HttpException>(&cause) else {
                    return Err(cause);
                };
                // If the error is due to an expired auth token and we can reset the input stream,
                // then try again. `reset(in)` succeeds only for a stream with `mark` support (or a
                // `FileInputStream`); the retry then sends the body again from its start (a new
                // stream of the blob).
                if self.auth_token_expired(http_exception) && blob.supports_reset() {
                    self.refresh_credentials()?;
                    // uploadAfterCredentialRefresh
                    return self.upload(key, length, blob, cas_upload);
                }
                Err(cause)
            }
        }
    }

    /// `acquireUploadChannel`, `HttpUploadHandler.write` and `channelRead0`,
    /// `releaseUploadChannel`.
    // Java: com/google/devtools/build/lib/remote/http/HttpUploadHandler.java:89 HttpUploadHandler.write
    // Java: com/google/devtools/build/lib/remote/http/HttpUploadHandler.java:54 HttpUploadHandler.channelRead0
    fn upload(&self, key: &str, length: i64, blob: &dyn Blob, cas_upload: bool) -> io::Result<()> {
        let path = construct_path(&self.uri, key, cas_upload);
        let mut headers: Vec<(String, String)> = Vec::new();
        // buildRequest(path, constructHost(uri), contentLength)
        set_header(&mut headers, "host", &construct_host(&self.uri));
        set_header(&mut headers, "accept", "*/*");
        set_header(&mut headers, "content-length", &length.to_string());
        set_header(&mut headers, "connection", "keep-alive");
        self.add_credential_headers(&mut headers)?;
        self.add_extra_remote_headers(&mut headers);
        self.add_user_agent_header(&mut headers);
        let request = request_head("PUT", &path, &headers);

        let connection = match self.pool.acquire()? {
            Some(connection) => connection,
            None => match self.connect() {
                Ok(connection) => connection,
                Err(e) => {
                    self.pool.release(None);
                    return Err(e);
                }
            },
        };
        match Self::exchange_upload(
            connection,
            &request,
            blob,
            &path,
            length,
            self.idle_timeout(),
        ) {
            Ok((kept, result)) => {
                self.pool.release(kept);
                result
            }
            Err(e) => {
                self.pool.release(None);
                Err(e)
            }
        }
    }

    /// Writes the request and its body, reads the response. `Ok((kept, result))` once a
    /// response was read, with the connection when it is kept alive.
    fn exchange_upload(
        mut connection: Connection,
        request: &[u8],
        blob: &dyn Blob,
        path: &str,
        length: i64,
        idle_timeout: Option<Duration>,
    ) -> io::Result<(Option<Connection>, io::Result<()>)> {
        let timeout = || -> io::Error { UploadTimeoutException::new(path, length).into() };
        let write_error = |e: io::Error| {
            if is_timeout(&e) {
                timeout()
            } else {
                closed_or(e)
            }
        };
        // The body: `HttpChunkedInput(new ChunkedStream(data))`, sent as is (the request has a
        // Content-Length, so the encoder adds no chunk framing). The response is read as soon as
        // the server sends one, whatever has been written (see `write_watching`).
        let stream = connection.get_mut();
        let mut answered_early = write_watching(stream, request, idle_timeout)
            .map_err(write_error)?
            == Written::Answered;
        if !answered_early {
            let mut data = blob.get()?;
            let mut buf = vec![0u8; 8192];
            loop {
                let n = match data.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) => return Err(e),
                };
                if write_watching(stream, &buf[..n], idle_timeout).map_err(write_error)?
                    == Written::Answered
                {
                    answered_early = true;
                    break;
                }
            }
        }

        let head = match read_response_head(&mut connection) {
            Ok(head) => head,
            Err(e) => {
                return Err(match head_failure(&e) {
                    HeadFailure::Timeout => timeout(),
                    HeadFailure::Closed => ClosedChannelException.into(),
                    HeadFailure::Parse => {
                        errors::io_exception("Failed to parse the HTTP response.")
                    }
                    HeadFailure::Other => io_from_repo_fetch(&e),
                });
            }
        };
        // HttpObjectAggregator: the whole body, at most 10 KiB.
        let framing = body_framing(&head);
        let mut body = BodyStream::new(connection, framing);
        let mut content: Vec<u8> = Vec::new();
        loop {
            let mut chunk = [0u8; 4096];
            let n = body.read(&mut chunk).map_err(|e| {
                if e.kind == IoExceptionKind::SocketTimeoutException {
                    timeout()
                } else if e.kind == IoExceptionKind::SocketException
                    || e.get_message() == "Premature EOF"
                {
                    ClosedChannelException.into()
                } else {
                    io_from_repo_fetch(&e)
                }
            })?;
            if n == 0 {
                break;
            }
            if content.len() + n > MAX_UPLOAD_RESPONSE_CONTENT {
                // TooLongFrameException, wrapped: `new IOException(t)`.
                // PORT: Netty's message also names the response; only its first part is kept.
                return Err(errors::io_exception(format!(
                    "io.netty.handler.codec.TooLongFrameException: Response entity too large: {}",
                    head.status_line
                )));
            }
            content.extend_from_slice(&chunk[..n]);
        }
        if let BodyFraming::Fixed(_) = framing
            && body.framing() != BodyFraming::Done
        {
            return Err(ClosedChannelException.into());
        }
        let Some(connection) = body.into_reader() else {
            return Err(ClosedChannelException.into());
        };
        // PORT: Java releases the channel to the pool after an early answer too, with part of the
        // body unwritten (its `ChunkedWriteHandler` is removed), so that its next request would
        // be read as the rest of this body; here such a connection is closed instead.
        let kept = (is_keep_alive(&head) && !answered_early).then_some(connection);
        // Supporting more than OK status to be compatible with nginx webdav.
        let result = if !matches!(head.code, 200 | 202 | 201 | 204) {
            let mut error_msg = format!("{} {}", head.code, head.message);
            if !content.is_empty() {
                error_msg.push('\n');
                error_msg.push_str(&decode_charset(&head, &content));
            }
            Err(http_exception(&head, error_msg).into())
        } else {
            Ok(())
        };
        Ok((kept, result))
    }

    /// `addCredentialHeaders(request, uri)`.
    // Java: com/google/devtools/build/lib/remote/http/AbstractHttpHandler.java:63 AbstractHttpHandler.addCredentialHeaders
    fn add_credential_headers(&self, headers: &mut Vec<(String, String)>) -> io::Result<()> {
        if let Some(user_info) = self.uri.get_user_info() {
            use base64::Engine as _;
            // BaseEncoding.base64Url(), with padding.
            let value = base64::engine::general_purpose::URL_SAFE.encode(user_info.as_bytes());
            set_header(headers, "authorization", &format!("Basic {value}"));
            return Ok(());
        }
        // `credentials == null || !credentials.hasRequestMetadata()`: every port of a
        // `Credentials` has request metadata.
        let Some(creds) = &self.creds else {
            return Ok(());
        };
        let auth_headers = creds
            .get_request_metadata(&self.uri)
            .map_err(|e| io_from_repo_fetch(&e))?;
        for (name, values) in auth_headers {
            for value in values {
                headers.push((name.clone(), value));
            }
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/remote/http/AbstractHttpHandler.java:85 AbstractHttpHandler.addExtraRemoteHeaders
    fn add_extra_remote_headers(&self, headers: &mut Vec<(String, String)>) {
        for (name, value) in &self.extra_http_headers {
            headers.push((name.clone(), value.clone()));
        }
    }

    // Java: com/google/devtools/build/lib/remote/http/AbstractHttpHandler.java:91 AbstractHttpHandler.addUserAgentHeader
    fn add_user_agent_header(&self, headers: &mut Vec<(String, String)>) {
        set_header(headers, "user-agent", &self.user_agent);
    }

    /// See <https://tools.ietf.org/html/rfc6750#section-3.1>.
    // Java: com/google/devtools/build/lib/remote/http/HttpCacheClient.java:799 HttpCacheClient.authTokenExpired
    fn auth_token_expired(&self, response: &HttpException) -> bool {
        if self.creds.is_none() {
            return false;
        }
        let value = response.header_values("www-authenticate").join(",");
        if value.starts_with("Bearer") {
            // INVALID_TOKEN_ERROR.matcher(value).find()
            static INVALID_TOKEN_ERROR: std::sync::LazyLock<regex::Regex> =
                std::sync::LazyLock::new(|| {
                    regex::Regex::new(r#"\s*error\s*=\s*"?invalid_token"?"#).unwrap()
                });
            INVALID_TOKEN_ERROR.is_match(&value)
        } else {
            response.status == 401
        }
    }

    // Java: com/google/devtools/build/lib/remote/http/HttpCacheClient.java:814 HttpCacheClient.refreshCredentials
    fn refresh_credentials(&self) -> io::Result<()> {
        let mut last_refresh_time = self.last_refresh_time.lock();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        // Call creds.refresh() at most once per second. The one second was arbitrarily chosen,
        // as a small enough value that we don't expect to interfere with actual token lifetimes,
        // but it should just make sure that potentially hundreds of threads don't call this
        // method at the same time.
        if now - *last_refresh_time > 1000 {
            *last_refresh_time = now;
            if let Some(creds) = &self.creds {
                creds.refresh()?;
            }
        }
        Ok(())
    }
}

/// The download handler's counters for its timeout message.
struct DownloadState {
    path: String,
    bytes_received: i64,
    content_length: i64,
}

/// `get`'s `wrappedOut`: notes that data was written and counts the CAS bytes for resuming.
struct CountingOut<'a> {
    out: &'a mut dyn Write,
    data_written: &'a AtomicBool,
    cas_bytes_downloaded: Option<&'a AtomicI64>,
}

impl Write for CountingOut<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.data_written.store(true, Ordering::SeqCst);
        let n = self.out.write(buf)?;
        if let Some(count) = self.cas_bytes_downloaded {
            count.fetch_add(n as i64, Ordering::SeqCst);
        }
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}

/// The body as `std::io::Read`: the stack's errors as `io::Error`, a connection that ends a
/// fixed-length body early as `ClosedChannelException` (Netty sees the channel go inactive).
struct RawBody<'a> {
    body: &'a mut BodyStream<Stream>,
    fixed_remaining: Option<u64>,
}

impl Read for RawBody<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.body.read(buf).map_err(|e| {
            if e.kind == IoExceptionKind::SocketTimeoutException {
                io::Error::new(io::ErrorKind::TimedOut, "Read timed out")
            } else if e.kind == IoExceptionKind::SocketException
                || e.get_message() == "Premature EOF"
            {
                ClosedChannelException.into()
            } else {
                io_from_repo_fetch(&e)
            }
        })?;
        if let Some(remaining) = &mut self.fixed_remaining {
            if n == 0 && *remaining > 0 {
                return Err(ClosedChannelException.into());
            }
            *remaining -= n as u64;
        }
        Ok(n)
    }
}

/// How `HttpContentDecompressor` decodes a response (`newContentDecoder`).
enum ContentDecoder {
    Gzip,
    Deflate,
    Snappy,
    Zstd,
}

/// The decoder of the response's `Content-Encoding`; `None`: the content is passed on as is.
// Java: io/netty/handler/codec/http/HttpContentDecompressor.java HttpContentDecompressor.newContentDecoder
fn content_decoder(head: &ResponseHead) -> io::Result<Option<ContentDecoder>> {
    let Some(encoding) = head.header("content-encoding") else {
        return Ok(None);
    };
    let encoding = encoding.trim();
    if encoding.eq_ignore_ascii_case("gzip") || encoding.eq_ignore_ascii_case("x-gzip") {
        Ok(Some(ContentDecoder::Gzip))
    } else if encoding.eq_ignore_ascii_case("deflate") || encoding.eq_ignore_ascii_case("x-deflate")
    {
        Ok(Some(ContentDecoder::Deflate))
    } else if encoding.eq_ignore_ascii_case("zstd") {
        Ok(Some(ContentDecoder::Zstd))
    } else if encoding.eq_ignore_ascii_case("snappy") {
        // `new SnappyFrameDecoder()`: checksums are not validated.
        Ok(Some(ContentDecoder::Snappy))
    } else {
        // `br` needs Brotli4j, which the oracle does not bundle; any other coding is left as is.
        Ok(None)
    }
}

/// `ZlibWrapper.ZLIB_OR_NONE`: a zlib stream when the first two bytes are a zlib header, raw
/// deflate otherwise.
fn zlib_or_none<'a>(raw: &'a mut RawBody<'_>) -> Box<dyn Read + 'a> {
    let mut reader = BufReader::new(raw);
    let is_zlib = match reader.fill_buf() {
        Ok([cmf, flg, ..]) => {
            cmf & 0x0f == 8 && (u16::from(*cmf) * 256 + u16::from(*flg)).is_multiple_of(31)
        }
        _ => false,
    };
    if is_zlib {
        Box::new(flate2::bufread::ZlibDecoder::new(reader))
    } else {
        Box::new(flate2::bufread::DeflateDecoder::new(reader))
    }
}

/// `HttpUtil.isKeepAlive(response)`.
fn is_keep_alive(head: &ResponseHead) -> bool {
    let connection = head.header_values("connection").join(",");
    let has_token = |token: &str| {
        connection
            .split(',')
            .any(|t| t.trim().eq_ignore_ascii_case(token))
    };
    if has_token("close") {
        return false;
    }
    if head.status_line.starts_with("HTTP/1.1") {
        true
    } else {
        has_token("keep-alive")
    }
}

/// `new String(bytes, HttpUtil.getCharset(response))`: the `charset` of the Content-Type, else
/// ISO-8859-1.
fn decode_charset(head: &ResponseHead, bytes: &[u8]) -> String {
    let charset = head.header("content-type").and_then(|ct| {
        ct.split(';').skip(1).find_map(|p| {
            let (k, v) = p.split_once('=')?;
            k.trim()
                .eq_ignore_ascii_case("charset")
                .then(|| v.trim().trim_matches('"').to_ascii_lowercase())
        })
    });
    match charset.as_deref() {
        Some("utf-8") | Some("utf8") => String::from_utf8_lossy(bytes).into_owned(),
        // PORT: other charsets than UTF-8 and ISO-8859-1 are read as ISO-8859-1.
        _ => bytes.iter().map(|&b| b as char).collect(),
    }
}

fn http_exception(head: &ResponseHead, message: String) -> HttpException {
    HttpException::new(
        head.code as u16,
        head.message.clone(),
        head.headers.clone(),
        message,
    )
}

/// `cacheMiss(status)`: supporting NO_CONTENT for nginx webdav compatibility.
// Java: com/google/devtools/build/lib/remote/http/HttpCacheClient.java:792 HttpCacheClient.cacheMiss
fn cache_miss(status: u16) -> bool {
    status == 404 || status == 204
}

/// `constructPath(uri, hash, isCas)`.
// Java: com/google/devtools/build/lib/remote/http/AbstractHttpHandler.java:95 AbstractHttpHandler.constructPath
fn construct_path(uri: &JavaUri, hash: &str, is_cas: bool) -> String {
    let path = uri.get_path().unwrap_or_default();
    let mut builder = String::new();
    builder.push_str(&path);
    if !path.ends_with('/') {
        builder.push('/');
    }
    builder.push_str(if is_cas { CAS_PREFIX } else { AC_PREFIX });
    builder.push_str(hash);
    builder
}

/// `constructHost(uri)`.
// Java: com/google/devtools/build/lib/remote/http/AbstractHttpHandler.java:106 AbstractHttpHandler.constructHost
fn construct_host(uri: &JavaUri) -> String {
    let port = uri.get_port();
    let scheme = uri.get_scheme().unwrap_or("");
    let include_port =
        port > 0 && ((scheme == "http" && port != 80) || (scheme == "https" && port != 443));
    let host = uri.get_host().unwrap_or("");
    if include_port {
        format!("{host}:{port}")
    } else {
        host.to_string()
    }
}

/// `headers.set(name, value)`: replaces every field of that name (case-insensitive), at the end.
fn set_header(headers: &mut Vec<(String, String)>, name: &str, value: &str) {
    headers.retain(|(k, _)| !k.eq_ignore_ascii_case(name));
    headers.push((name.to_string(), value.to_string()));
}

/// `HttpRequestEncoder`: the request line and the header fields (ISO-8859-1).
fn request_head(method: &str, path: &str, headers: &[(String, String)]) -> Vec<u8> {
    let mut head = format!("{method} {path} HTTP/1.1\r\n");
    for (name, value) in headers {
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    blaze_util::string_encoding::latin1_encode_lossy(&head).into_owned()
}

/// How a watched write ended.
#[derive(Debug, PartialEq, Eq)]
enum Written {
    /// Everything was written (and, for TLS, sent).
    All,
    /// The server answered (or closed the connection) before; the rest was not written.
    Answered,
}

/// `poll(2)` of one descriptor; `Ok(0)` on timeout. `timeout` `None` waits forever.
fn poll_fd(fd: std::os::fd::RawFd, events: i16, timeout: Option<Duration>) -> io::Result<i16> {
    let timeout_ms = match timeout {
        None => -1,
        Some(t) => i32::try_from(t.as_millis()).unwrap_or(i32::MAX),
    };
    let mut pollfd = libc::pollfd {
        fd,
        events,
        revents: 0,
    };
    loop {
        // SAFETY: one valid pollfd for the duration of the call.
        let n = unsafe { libc::poll(&mut pollfd, 1, timeout_ms) };
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        return Ok(if n == 0 { 0 } else { pollfd.revents });
    }
}

/// The time left until `deadline` (`None`: no timeout).
fn remaining(deadline: Option<Instant>) -> Option<Duration> {
    deadline.map(|d| d.saturating_duration_since(Instant::now()))
}

/// Writes `data` on a plain socket while watching for the server's answer.
///
/// As Netty's `NioSocketChannel.doWrite`: the data is written as long as the socket takes it,
/// and the channel waits for writability (`OP_WRITE`) only once a write takes nothing. (Waiting
/// for `POLLOUT` before each write would stop at half the send buffer: Linux reports a TCP
/// socket writable only while its free space is at least half of what is queued.) The
/// `IdleTimeoutHandler` restarts its timer when a write completes (`IdleStateHandler`'s write
/// listener), not on partial progress: `timeout` runs from the start of this piece.
// Java: io/netty/channel/socket/nio/NioSocketChannel.java NioSocketChannel.doWrite
// Java: io/netty/handler/timeout/IdleStateHandler.java IdleStateHandler.write
fn write_watching_plain<S: Write + std::os::fd::AsRawFd>(
    sock: &mut S,
    set_nonblocking: &dyn Fn(&S, bool) -> io::Result<()>,
    data: &[u8],
    timeout: Option<Duration>,
) -> io::Result<Written> {
    let deadline = timeout.map(|t| Instant::now() + t);
    set_nonblocking(sock, true)?;
    let result = (|| {
        let mut off = 0;
        while off < data.len() {
            // The event loop reads whatever the server sends while it writes.
            if poll_fd(sock.as_raw_fd(), libc::POLLIN, Some(Duration::ZERO))? & libc::POLLIN != 0 {
                return Ok(Written::Answered);
            }
            match sock.write(&data[off..]) {
                Ok(n) => {
                    off += n;
                    continue;
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(e) => return Err(e),
            }
            let revents = poll_fd(
                sock.as_raw_fd(),
                libc::POLLIN | libc::POLLOUT,
                remaining(deadline),
            )?;
            if revents == 0 {
                return Err(io::Error::from(io::ErrorKind::TimedOut));
            }
            if revents & libc::POLLIN != 0 {
                return Ok(Written::Answered);
            }
        }
        Ok(Written::All)
    })();
    set_nonblocking(sock, false)?;
    result
}

/// Writes `data` on a TLS connection while watching for the server's answer: received records
/// count as an answer once they decrypt to application data or close the connection (a TLS 1.3
/// session ticket is not an answer). Plaintext that arrives stays in the connection for the
/// response reader. Writes go out as long as the socket takes them, and `timeout` runs from the
/// start of the piece, as for [`write_watching_plain`].
fn write_watching_tls<S: Read + Write + std::os::fd::AsRawFd>(
    tls: &mut StreamOwned<ClientConnection, S>,
    set_nonblocking: &dyn Fn(&S, bool) -> io::Result<()>,
    data: &[u8],
    timeout: Option<Duration>,
) -> io::Result<Written> {
    let deadline = timeout.map(|t| Instant::now() + t);
    set_nonblocking(&tls.sock, true)?;
    let result = (|| {
        let mut off = 0;
        loop {
            while off < data.len() {
                let n = tls.conn.writer().write(&data[off..])?;
                if n == 0 {
                    break;
                }
                off += n;
            }
            // Write the records while the socket takes them.
            while tls.conn.wants_write() {
                match tls.conn.write_tls(&mut tls.sock) {
                    Ok(_) => {}
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) => return Err(e),
                }
            }
            let blocked = tls.conn.wants_write();
            if !blocked && off == data.len() {
                return Ok(Written::All);
            }
            // Blocked on the socket: wait for writability or an answer until the deadline.
            // Otherwise only look for an answer.
            let (events, wait) = if blocked {
                (libc::POLLIN | libc::POLLOUT, remaining(deadline))
            } else {
                (libc::POLLIN, Some(Duration::ZERO))
            };
            let revents = poll_fd(tls.sock.as_raw_fd(), events, wait)?;
            if blocked && revents == 0 {
                return Err(io::Error::from(io::ErrorKind::TimedOut));
            }
            if revents & libc::POLLIN != 0 {
                match tls.conn.read_tls(&mut tls.sock) {
                    Ok(0) => return Ok(Written::Answered),
                    Ok(_) => {
                        let state = tls.conn.process_new_packets().map_err(io::Error::other)?;
                        if state.plaintext_bytes_to_read() > 0 || state.peer_has_closed() {
                            return Ok(Written::Answered);
                        }
                    }
                    Err(e)
                        if matches!(
                            e.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                        ) => {}
                    Err(e) => return Err(e),
                }
            }
        }
    })();
    set_nonblocking(&tls.sock, false)?;
    result
}

/// Writes part of an upload while reading for the answer, as Netty's event loop reads the
/// response (`HttpUploadHandler.channelRead0`) while `ChunkedWriteHandler` writes the body: a
/// server may answer (a 401, 403 or 413) before it has read the body. A timeout is a
/// `TimedOut` error: no read or write within `timeout` (the `IdleTimeoutHandler`'s all-idle
/// timer of the socket timeouts).
// Java: com/google/devtools/build/lib/remote/http/HttpUploadHandler.java:54 HttpUploadHandler.channelRead0
fn write_watching(
    stream: &mut Stream,
    data: &[u8],
    timeout: Option<Duration>,
) -> io::Result<Written> {
    match stream {
        Stream::Tcp(s) => write_watching_plain(s, &|s, b| s.set_nonblocking(b), data, timeout),
        Stream::Unix(s) => write_watching_plain(s, &|s, b| s.set_nonblocking(b), data, timeout),
        Stream::TlsTcp(s) => write_watching_tls(s, &|s, b| s.set_nonblocking(b), data, timeout),
        Stream::TlsUnix(s) => write_watching_tls(s, &|s, b| s.set_nonblocking(b), data, timeout),
    }
}

fn write_all_flush(stream: &mut Stream, bytes: &[u8]) -> io::Result<()> {
    stream.write_all(bytes)?;
    stream.flush()
}

fn is_timeout(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    )
}

/// A write on a connection the peer closed: `ClosedChannelException`.
fn closed_or(e: io::Error) -> io::Error {
    match e.kind() {
        io::ErrorKind::BrokenPipe
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::ConnectionAborted
        | io::ErrorKind::UnexpectedEof => ClosedChannelException.into(),
        _ => e,
    }
}

enum HeadFailure {
    Timeout,
    Closed,
    Parse,
    Other,
}

/// What a failure to read a response head stands for in Netty.
fn head_failure(e: &IoException) -> HeadFailure {
    if e.kind == IoExceptionKind::SocketTimeoutException {
        HeadFailure::Timeout
    } else if e.kind == IoExceptionKind::SocketException {
        // "Unexpected end of file from server" (the connection closed before a response),
        // "Connection reset".
        HeadFailure::Closed
    } else if e.get_message().starts_with("Invalid Http response")
        || e.get_message().starts_with("HTTP header line is too long")
        || e.get_message().starts_with("Too many HTTP header fields")
    {
        // decoderResult() not successful.
        HeadFailure::Parse
    } else {
        HeadFailure::Other
    }
}

fn io_from_repo_fetch(e: &IoException) -> io::Error {
    errors::io_exception(e.get_message())
}

/// The message the JDK gives a socket error (`strerror` without Rust's " (os error N)").
fn java_io_message(e: &io::Error) -> String {
    let message = e.to_string();
    match message.find(" (os error ") {
        Some(i) => message[..i].to_string(),
        None => message,
    }
}

/// `java.net.UnknownHostException` of a host that does not resolve.
fn unknown_host(host: &str, e: &io::Error) -> io::Error {
    let _ = e;
    io::Error::new(
        io::ErrorKind::NotFound,
        errors::JavaException {
            class_name: "java.net.UnknownHostException",
            message: host.to_string(),
        },
    )
}

/// Netty's `AnnotatedConnectException`: `<reason>: <host>/<address>`.
fn connect_exception(e: &io::Error, host: &str, address: &SocketAddr) -> io::Error {
    let reason = if e.kind() == io::ErrorKind::TimedOut {
        "connection timed out".to_string()
    } else {
        java_io_message(e)
    };
    io::Error::new(
        e.kind(),
        errors::JavaException {
            class_name: "io.netty.channel.AbstractChannel$AnnotatedConnectException",
            message: format!("{reason}: {host}/{address}"),
        },
    )
}

/// The host the TLS engine is created for (`sslCtx.newEngine(alloc, hostname, port)`).
fn host_for_tls(uri: &JavaUri) -> String {
    uri.get_host().unwrap_or_default().to_string()
}

fn client_connection(tls: &Arc<ClientConfig>, host: &str) -> io::Result<ClientConnection> {
    let server_name =
        ServerName::try_from(host.to_string()).map_err(|e| errors::io_exception(format!("{e}")))?;
    ClientConnection::new(tls.clone(), server_name).map_err(io::Error::other)
}

/// A spawned task that is aborted when its handle is dropped.
struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);

impl<T> std::future::Future for AbortOnDrop<T> {
    type Output = T;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<T> {
        std::pin::Pin::new(&mut self.0)
            .poll(cx)
            .map(|result| match result {
                Ok(value) => value,
                Err(e) if e.is_panic() => std::panic::resume_unwind(e.into_panic()),
                Err(e) => panic!("upload task cancelled: {e}"),
            })
    }
}

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Runs blocking network I/O from an async method: on the remote runtime's worker thread
/// (`block_in_place`), or directly on a thread outside a multi-threaded runtime.
fn blocking<T>(f: impl FnOnce() -> T) -> T {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(f)
        }
        _ => f(),
    }
}

#[async_trait]
impl MissingDigestsFinder for HttpCacheClient {
    // Java: com/google/devtools/build/lib/remote/http/HttpCacheClient.java:729 HttpCacheClient.findMissingDigests
    async fn find_missing_digests(
        &self,
        _context: &RemoteActionExecutionContext<'_>,
        digests: &[Digest],
    ) -> io::Result<IndexSet<Digest>> {
        Ok(digests.iter().cloned().collect())
    }
}

#[async_trait]
impl RemoteCacheClient for HttpCacheClient {
    /// Each upload as a task of its own on the remote runtime: its network I/O blocks the task
    /// (`blocking`), and the uploads a caller waits for together (the outputs of an action)
    /// proceed at once, as each upload has its own channel in Java. Dropping the returned future
    /// aborts the task (at its next await; a transfer in progress ends with its socket timeout).
    fn upload_blob_owned(
        self: Arc<Self>,
        context: RemoteActionExecutionContext<'static>,
        digest: Digest,
        blob: Arc<dyn Blob>,
    ) -> futures::future::BoxFuture<'static, io::Result<()>> {
        let task = crate::runtime::runtime()
            .spawn(async move { self.upload_blob_impl(&context, &digest, &*blob).await });
        Box::pin(AbortOnDrop(task))
    }

    fn cas_upload_cache(&self) -> &AsyncTaskCacheNoResult<Digest> {
        &self.cas_upload_cache
    }

    // Java: com/google/devtools/build/lib/remote/http/HttpCacheClient.java:605 HttpCacheClient.getServerCapabilities
    fn get_server_capabilities(&self) -> io::Result<ServerCapabilities> {
        let cache_capabilities = CacheCapabilities {
            action_cache_update_capabilities: Some(ActionCacheUpdateCapabilities {
                update_enabled: true,
            }),
            symlink_absolute_path_strategy: symlink_absolute_path_strategy::Value::Allowed as i32,
            ..Default::default()
        };
        Ok(ServerCapabilities {
            cache_capabilities: Some(cache_capabilities),
            ..Default::default()
        })
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    // Java: com/google/devtools/build/lib/remote/http/HttpCacheClient.java:616 HttpCacheClient.getAuthority
    async fn get_authority(&self) -> io::Result<String> {
        Ok(String::new())
    }

    // Java: com/google/devtools/build/lib/remote/http/HttpCacheClient.java:621 HttpCacheClient.downloadActionResult
    async fn download_action_result(
        &self,
        _context: &RemoteActionExecutionContext<'_>,
        action_key: &ActionKey,
        _inline_out_err: bool,
        _inline_output_files: &IndexSet<String>,
    ) -> io::Result<Option<ActionResult>> {
        self.retrier
            .execute_async(|| {
                utils::download_as_action_result(action_key, |digest, data| async move {
                    let mut data = data;
                    blocking(|| {
                        self.get(&digest, &mut data, /* cas_bytes_downloaded= */ None)
                    })?;
                    Ok(data)
                })
            })
            .await
    }

    // Java: com/google/devtools/build/lib/remote/http/HttpCacheClient.java:748 HttpCacheClient.uploadActionResult
    async fn upload_action_result(
        &self,
        _context: &RemoteActionExecutionContext<'_>,
        action_key: &ActionKey,
        action_result: &ActionResult,
    ) -> io::Result<()> {
        let serialized = bytes::Bytes::from(action_result.encode_to_vec());
        let length = serialized.len() as i64;
        let blob = crate::common::remote_cache_client::BytesBlob(serialized);
        blocking(|| {
            self.upload_async(
                &action_key.digest().hash,
                length,
                &blob,
                /* cas_upload= */ false,
            )
        })
    }

    // Java: com/google/devtools/build/lib/remote/http/HttpCacheClient.java:455 HttpCacheClient.downloadBlob
    async fn download_blob(
        &self,
        _context: &RemoteActionExecutionContext<'_>,
        digest: &Digest,
        out: &mut (dyn Write + Send),
    ) -> io::Result<()> {
        let cas_bytes_downloaded = AtomicI64::new(0);
        let cas_bytes_downloaded = &cas_bytes_downloaded;
        if self.verify_downloads {
            let digest_out = Mutex::new(self.digest_util.new_digest_output_stream(&mut *out));
            let target = &digest_out;
            self.retrier
                .execute_async(|| async move {
                    blocking(|| self.get(digest, &mut *target.lock(), Some(cas_bytes_downloaded)))
                })
                .await?;
            // Utils.verifyBlobContents(digest, digestOut.digest())
            let actual = digest_out.into_inner().digest();
            utils::verify_blob_contents(digest, &actual)?;
        } else {
            let target = Mutex::new(&mut *out);
            let target = &target;
            self.retrier
                .execute_async(|| async move {
                    blocking(|| {
                        let mut out = target.lock();
                        self.get(digest, &mut **out, Some(cas_bytes_downloaded))
                    })
                })
                .await?;
        }
        out.flush()
    }

    // Java: com/google/devtools/build/lib/remote/http/HttpCacheClient.java:720 HttpCacheClient.uploadBlobImpl
    async fn upload_blob_impl(
        &self,
        _context: &RemoteActionExecutionContext<'_>,
        digest: &Digest,
        blob: &dyn Blob,
    ) -> io::Result<()> {
        self.retrier
            .execute_async(|| async {
                blocking(|| {
                    self.upload_async(
                        &digest.hash,
                        digest.size_bytes,
                        blob,
                        /* cas_upload= */ true,
                    )
                })
            })
            .await
    }

    // Java: com/google/devtools/build/lib/remote/http/HttpCacheClient.java:764 HttpCacheClient.close
    fn close(&self) {
        if self.is_closed.swap(true, Ordering::SeqCst) {
            return;
        }
        self.pool.close();
    }
}

impl std::fmt::Debug for HttpCacheClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpCacheClient")
            .field("uri", &self.uri.as_str())
            .field("use_tls", &self.use_tls)
            .finish_non_exhaustive()
    }
}
