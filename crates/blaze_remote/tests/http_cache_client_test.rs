//! Port of `src/test/java/com/google/devtools/build/lib/remote/http/HttpCacheClientTest.java`.
//!
//! The Java test is parameterized over a server on an internet socket and one on a Unix domain
//! socket (Epoll); each test here runs on both (`_inet` and `_unix`). The Java servers are Netty
//! pipelines (`HttpServerCodec`, `HttpObjectAggregator`, a handler); here a thread per
//! connection reads each request (head and Content-Length body) and the test's handler answers
//! with raw response bytes, keeping or closing the connection as the Java handler's
//! `ChannelFutureListener.CLOSE` does.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use blaze_optdefs::misc::auth_and_tls_options::AuthAndTLSOptions;
use blaze_remote::common::cache_not_found_exception::CacheNotFoundException;
use blaze_remote::common::errors;
use blaze_remote::common::{
    ActionKey, RemoteActionExecutionContext, RemoteCacheClient, RemoteCacheClientUploads,
};
use blaze_remote::http::http_cache_client::HttpCacheClient;
use blaze_remote::http::{
    ClosedChannelException, DownloadTimeoutException, HttpException, UploadTimeoutException,
};
use blaze_remote::reapi::{ActionResult, Digest, OutputFile, RequestMetadata};
use blaze_remote::remote_retrier::RemoteRetrier;
use blaze_remote::retrier::{
    ALLOW_ALL_CALLS, Backoff, RETRIES_DISABLED, ResultClassifierResult, TokioRetryScheduler,
    ZeroBackoff,
};
use blaze_remote::runtime::get_from_future;
use blaze_remote::util::DigestUtil;
use blaze_repo_fetch::authandtls::Headers;
use blaze_repo_fetch::errors::IoException;
use blaze_repo_fetch::uri::JavaUri;
use blaze_vfs::DigestHashFunction;
use prost::Message;

fn digest_util() -> DigestUtil {
    DigestUtil::new(DigestHashFunction::Sha256)
}

/// `DIGEST`: the digest of "File Contents".
fn digest() -> Digest {
    digest_util().compute_as_utf8("File Contents")
}

fn context() -> RemoteActionExecutionContext<'static> {
    RemoteActionExecutionContext::create(RequestMetadata::default())
}

/// A request as the test server read it.
#[derive(Debug, Clone)]
struct Request {
    method: String,
    path: String,
    /// Lower-cased names.
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

/// What the handler does with a request.
enum Answer {
    /// Writes these bytes and keeps the connection open.
    Respond(Vec<u8>),
    /// Writes these bytes and closes the connection (`ChannelFutureListener.CLOSE`).
    RespondAndClose(Vec<u8>),
    /// Never answers (the connection stays open, idle).
    Silent,
}

type Handler = Arc<dyn Fn(&Request) -> Answer + Send + Sync>;

/// Consulted with the request head (an empty body) before the body is read: `Some` answers at
/// once, without reading the body, and keeps the connection open without reading from it.
type EarlyHandler = Arc<dyn Fn(&Request) -> Option<Answer> + Send + Sync>;

#[derive(Clone, Copy, Debug)]
enum ServerKind {
    Inet,
    Unix,
    /// An internet socket speaking TLS (the `testdata/mtls` server certificate for `localhost`).
    Tls,
}

struct TestServer {
    port: Option<u16>,
    socket: Option<PathBuf>,
    tls: bool,
}

fn mtls_file(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/testdata/mtls")
        .join(name)
}

/// The server side of [`ServerKind::Tls`].
fn tls_server_config() -> Arc<tokio_rustls::rustls::ServerConfig> {
    use tokio_rustls::rustls::pki_types::pem::PemObject;
    use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
    let certs: Vec<CertificateDer<'static>> =
        CertificateDer::pem_file_iter(mtls_file("server.crt"))
            .unwrap()
            .map(|c| c.unwrap())
            .collect();
    let key = PrivateKeyDer::from_pem_file(mtls_file("server.key")).unwrap();
    Arc::new(
        tokio_rustls::rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .unwrap(),
    )
}

impl Connection
    for tokio_rustls::rustls::StreamOwned<tokio_rustls::rustls::ServerConnection, TcpStream>
{
}

trait Connection: Read + Write + Send {}
impl Connection for TcpStream {}
impl Connection for UnixStream {}

fn read_request(reader: &mut BufReader<Box<dyn Connection>>) -> Option<Request> {
    let mut request = read_request_head(reader)?;
    read_request_body(reader, &mut request)?;
    Some(request)
}

fn read_request_body(
    reader: &mut BufReader<Box<dyn Connection>>,
    request: &mut Request,
) -> Option<()> {
    let length: usize = request
        .headers
        .iter()
        .find(|(k, _)| k == "content-length")
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).ok()?;
    request.body = body;
    Some(())
}

fn read_request_head(reader: &mut BufReader<Box<dyn Connection>>) -> Option<Request> {
    let mut line = String::new();
    if reader.read_line(&mut line).ok()? == 0 {
        return None;
    }
    let mut parts = line.trim_end().splitn(3, ' ');
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    let mut headers = Vec::new();
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).ok()?;
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        let (k, v) = header.split_once(':')?;
        headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
    }
    Some(Request {
        method,
        path,
        headers,
        body: Vec::new(),
    })
}

fn serve(stream: Box<dyn Connection>, handler: Handler, early: Option<EarlyHandler>) {
    let mut reader = BufReader::new(stream);
    loop {
        let Some(mut request) = read_request_head(&mut reader) else {
            return;
        };
        if let Some(early) = &early
            && let Some(answer) = early(&request)
        {
            let bytes = match answer {
                Answer::Respond(bytes) | Answer::RespondAndClose(bytes) => bytes,
                Answer::Silent => Vec::new(),
            };
            let _ = reader.get_mut().write_all(&bytes);
            // The body is never read; the connection stays open.
            std::thread::sleep(Duration::from_secs(5));
            return;
        }
        if read_request_body(&mut reader, &mut request).is_none() {
            return;
        }
        match handler(&request) {
            Answer::Respond(bytes) => {
                if reader.get_mut().write_all(&bytes).is_err() {
                    return;
                }
            }
            Answer::RespondAndClose(bytes) => {
                let _ = reader.get_mut().write_all(&bytes);
                return;
            }
            Answer::Silent => {
                std::thread::sleep(Duration::from_secs(5));
                return;
            }
        }
    }
}

impl TestServer {
    fn start(kind: ServerKind, handler: Handler) -> TestServer {
        TestServer::start_with_early(kind, handler, None)
    }

    fn start_with_early(
        kind: ServerKind,
        handler: Handler,
        early: Option<EarlyHandler>,
    ) -> TestServer {
        match kind {
            ServerKind::Tls => {
                let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
                let port = listener.local_addr().unwrap().port();
                let config = tls_server_config();
                std::thread::spawn(move || {
                    for stream in listener.incoming() {
                        let Ok(stream) = stream else { return };
                        let handler = handler.clone();
                        let early = early.clone();
                        let connection =
                            tokio_rustls::rustls::ServerConnection::new(config.clone()).unwrap();
                        let tls = tokio_rustls::rustls::StreamOwned::new(connection, stream);
                        std::thread::spawn(move || serve(Box::new(tls), handler, early));
                    }
                });
                TestServer {
                    port: Some(port),
                    socket: None,
                    tls: true,
                }
            }
            ServerKind::Inet => {
                let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
                let port = listener.local_addr().unwrap().port();
                std::thread::spawn(move || {
                    for stream in listener.incoming() {
                        let Ok(stream) = stream else { return };
                        let handler = handler.clone();
                        let early = early.clone();
                        std::thread::spawn(move || serve(Box::new(stream), handler, early));
                    }
                });
                TestServer {
                    port: Some(port),
                    socket: None,
                    tls: false,
                }
            }
            ServerKind::Unix => {
                // `File.createTempFile("bazel", ".sock", new File("/tmp"))`, deleted.
                static NEXT: AtomicUsize = AtomicUsize::new(0);
                let path = std::env::temp_dir().join(format!(
                    "blaze-http-cache-test-{}-{}.sock",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::SeqCst)
                ));
                let _ = std::fs::remove_file(&path);
                let listener = UnixListener::bind(&path).unwrap();
                std::thread::spawn(move || {
                    for stream in listener.incoming() {
                        let Ok(stream) = stream else { return };
                        let handler = handler.clone();
                        let early = early.clone();
                        std::thread::spawn(move || serve(Box::new(stream), handler, early));
                    }
                });
                TestServer {
                    port: None,
                    socket: Some(path),
                    tls: false,
                }
            }
        }
    }

    /// A server whose socket is already gone (`testServer.stop(server)` before the client
    /// connects).
    fn stopped(kind: ServerKind) -> TestServer {
        match kind {
            ServerKind::Inet | ServerKind::Tls => {
                let port = TcpListener::bind(("127.0.0.1", 0))
                    .unwrap()
                    .local_addr()
                    .unwrap()
                    .port();
                TestServer {
                    port: Some(port),
                    socket: None,
                    tls: false,
                }
            }
            ServerKind::Unix => TestServer {
                port: None,
                socket: Some(std::env::temp_dir().join(format!(
                    "blaze-http-cache-test-{}-stopped.sock",
                    std::process::id()
                ))),
                tls: false,
            },
        }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(socket) = &self.socket {
            let _ = std::fs::remove_file(socket);
        }
    }
}

/// `newCredentials()`: "Bearer invalidToken" until `refresh`, then "Bearer validToken".
#[derive(Default)]
struct MockCredentials {
    refreshed: Mutex<bool>,
    refresh_count: AtomicUsize,
    get_request_metadata_count: AtomicUsize,
}

impl blaze_repo_fetch::authandtls::Credentials for MockCredentials {
    fn get_request_metadata(&self, _uri: &JavaUri) -> Result<Headers, IoException> {
        self.get_request_metadata_count
            .fetch_add(1, Ordering::SeqCst);
        let token = if *self.refreshed.lock().unwrap() {
            "Bearer validToken"
        } else {
            "Bearer invalidToken"
        };
        let mut headers = Headers::new();
        headers.insert("Authorization".to_string(), vec![token.to_string()]);
        Ok(headers)
    }
}

impl blaze_remote::authandtls::Credentials for MockCredentials {
    fn refresh(&self) -> io::Result<()> {
        self.refresh_count.fetch_add(1, Ordering::SeqCst);
        *self.refreshed.lock().unwrap() = true;
        Ok(())
    }
}

/// A retrier without retries whose classifier reports success
/// (`createHttpBlobStore`'s default).
fn default_retrier() -> Arc<RemoteRetrier> {
    Arc::new(RemoteRetrier::with_backoff(
        Arc::new(|| -> Arc<dyn Backoff> { RETRIES_DISABLED.clone() }),
        Arc::new(|_e: &io::Error| ResultClassifierResult::Success),
        Arc::new(TokioRetryScheduler),
        ALLOW_ALL_CALLS.clone(),
    ))
}

/// One retry of a `ClosedChannelException`.
fn closed_channel_retrier() -> Arc<RemoteRetrier> {
    Arc::new(RemoteRetrier::with_backoff(
        Arc::new(|| -> Arc<dyn Backoff> { Arc::new(ZeroBackoff::new(1)) }),
        Arc::new(|e: &io::Error| {
            if errors::downcast_ref::<ClosedChannelException>(e).is_some() {
                ResultClassifierResult::TransientFailure
            } else {
                ResultClassifierResult::PermanentFailure
            }
        }),
        Arc::new(TokioRetryScheduler),
        ALLOW_ALL_CALLS.clone(),
    ))
}

/// The socket, the cache URI and, for a TLS server, the client's TLS configuration (trusting the
/// `testdata/mtls` CA, as `--tls_certificate` does).
fn server_address(
    server: &TestServer,
) -> (
    Option<PathBuf>,
    String,
    Option<Arc<tokio_rustls::rustls::ClientConfig>>,
) {
    match (&server.socket, server.port) {
        (Some(socket), _) => (Some(socket.clone()), "http://localhost".to_string(), None),
        (None, Some(port)) if server.tls => {
            let mut options = AuthAndTLSOptions::default();
            options.tls_certificate = Some(mtls_file("ca.crt").to_str().unwrap().to_string());
            let config = blaze_remote::authandtls::google_auth_utils::http_cache_tls_config(
                "localhost",
                &options,
            )
            .unwrap();
            (
                None,
                format!("https://localhost:{port}"),
                Some(Arc::new(config)),
            )
        }
        (None, Some(port)) => (None, format!("http://localhost:{port}"), None),
        (None, None) => unreachable!(),
    }
}

// Java: src/test/java/com/google/devtools/build/lib/remote/http/HttpCacheClientTest.java:274 HttpCacheClientTest.createHttpBlobStore
fn create_http_blob_store(
    server: &TestServer,
    timeout_seconds: i32,
    remote_verify_downloads: bool,
    extra_http_headers: Vec<(String, String)>,
    creds: Option<Arc<dyn blaze_remote::authandtls::Credentials>>,
    retrier: Option<Arc<RemoteRetrier>>,
) -> Arc<HttpCacheClient> {
    let retrier = retrier.unwrap_or_else(default_retrier);
    let (socket, uri, tls) = server_address(server);
    Arc::new(
        HttpCacheClient::create(
            socket,
            JavaUri::create(&uri).unwrap(),
            timeout_seconds,
            /* remote_max_connections= */ 0,
            remote_verify_downloads,
            extra_http_headers,
            digest_util(),
            retrier,
            creds,
            tls,
            "9.3.2",
        )
        .unwrap(),
    )
}

fn simple_store(
    server: &TestServer,
    creds: Option<Arc<dyn blaze_remote::authandtls::Credentials>>,
) -> Arc<HttpCacheClient> {
    create_http_blob_store(server, 1, true, Vec::new(), creds, None)
}

fn response(status: &str, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
    let mut out = format!("HTTP/1.1 {status}\r\n");
    for (k, v) in headers {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    out.push_str("\r\n");
    let mut out = out.into_bytes();
    out.extend_from_slice(body);
    out
}

fn ok_with_body(body: &[u8]) -> Vec<u8> {
    response(
        "200 OK",
        &[("content-length", &body.len().to_string())],
        body,
    )
}

fn download(store: &Arc<HttpCacheClient>, digest: &Digest) -> (io::Result<()>, Vec<u8>) {
    let mut out: Vec<u8> = Vec::new();
    let result = get_from_future(store.download_blob(&context(), digest, &mut out));
    (result, out)
}

/// `InMemoryHttpCacheServerHandler`: GET of a stored blob, PUT stores.
fn in_memory_cache(contents: Arc<Mutex<HashMap<String, Vec<u8>>>>) -> Handler {
    Arc::new(move |request: &Request| {
        let mut contents = contents.lock().unwrap();
        match request.method.as_str() {
            "PUT" => {
                contents.insert(request.path.clone(), request.body.clone());
                Answer::Respond(response("200 OK", &[("content-length", "0")], b""))
            }
            "GET" => match contents.get(&request.path) {
                Some(data) => Answer::Respond(ok_with_body(data)),
                None => Answer::Respond(response("404 Not Found", &[("content-length", "0")], b"")),
            },
            _ => Answer::RespondAndClose(response("400 Bad Request", &[], b"")),
        }
    })
}

macro_rules! both_servers {
    ($name:ident) => {
        mod $name {
            #[test]
            fn inet() {
                super::$name(super::ServerKind::Inet);
            }

            #[test]
            fn unix() {
                super::$name(super::ServerKind::Unix);
            }
        }
    };
}

// Java: src/test/java/com/google/devtools/build/lib/remote/http/HttpCacheClientTest.java:380 HttpCacheClientTest.testUpload
fn test_upload(kind: ServerKind) {
    let contents = Arc::new(Mutex::new(HashMap::new()));
    let server = TestServer::start(kind, in_memory_cache(contents.clone()));
    let blob_store = simple_store(&server, None);

    let data = bytes::Bytes::from_static(b"foo bar");
    let digest = digest_util().compute(&data);
    get_from_future(blob_store.upload_bytes(&context(), &digest, data.clone(), false)).unwrap();

    let contents = contents.lock().unwrap();
    assert_eq!(contents.len(), 1);
    let cache_key = format!("/cas/{}", digest.hash);
    assert_eq!(contents.get(&cache_key).map(Vec::as_slice), Some(&data[..]));
}
both_servers!(test_upload);

// Java: src/test/java/com/google/devtools/build/lib/remote/http/HttpCacheClientTest.java:404 HttpCacheClientTest.connectTimeout
fn connect_timeout(kind: ServerKind) {
    let server = TestServer::stopped(kind);
    let blob_store = simple_store(&server, Some(Arc::new(MockCredentials::default())));
    let (result, _) = download(&blob_store, &digest());
    let e = result.unwrap_err();
    // ConnectException: refused (a TCP port nobody listens on, a socket file that is gone).
    assert!(
        matches!(
            e.kind(),
            io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound
        ) || e.to_string().contains("connect"),
        "{e:?}"
    );
}
both_servers!(connect_timeout);

// Java: src/test/java/com/google/devtools/build/lib/remote/http/HttpCacheClientTest.java:421 HttpCacheClientTest.uploadTimeout
fn upload_timeout(kind: ServerKind) {
    // Don't respond and force a client timeout.
    let server = TestServer::start(kind, Arc::new(|_r: &Request| Answer::Silent));
    let blob_store = simple_store(&server, Some(Arc::new(MockCredentials::default())));
    let data = bytes::Bytes::from_static(b"File Contents");
    let e = get_from_future(blob_store.upload_bytes(
        &context(),
        &digest_util().compute(&data),
        data,
        false,
    ))
    .unwrap_err();
    assert!(
        errors::find_cause::<UploadTimeoutException>(&e).is_some()
            || errors::downcast_ref::<UploadTimeoutException>(&e).is_some(),
        "{e:?}"
    );
}
both_servers!(upload_timeout);

// Java: src/test/java/com/google/devtools/build/lib/remote/http/HttpCacheClientTest.java:454 HttpCacheClientTest.downloadTimeout
fn download_timeout(kind: ServerKind) {
    // Don't respond and force a client timeout.
    let server = TestServer::start(kind, Arc::new(|_r: &Request| Answer::Silent));
    let blob_store = simple_store(&server, Some(Arc::new(MockCredentials::default())));
    let (result, _) = download(&blob_store, &digest());
    let e = result.unwrap_err();
    assert!(
        errors::downcast_ref::<DownloadTimeoutException>(&e).is_some(),
        "{e:?}"
    );
}
both_servers!(download_timeout);

// Java: src/test/java/com/google/devtools/build/lib/remote/http/HttpCacheClientTest.java:483 HttpCacheClientTest.uploadResponseTooLarge
fn upload_response_too_large(kind: ServerKind) {
    let server = TestServer::start(
        kind,
        Arc::new(|_r: &Request| {
            let long_message = vec![0u8; 50000];
            Answer::RespondAndClose(response(
                "500 Internal Server Error",
                &[("content-length", "50000")],
                &long_message,
            ))
        }),
    );
    let blob_store = simple_store(&server, Some(Arc::new(MockCredentials::default())));
    let data = bytes::Bytes::from_static(b"File Contents");
    let e = get_from_future(blob_store.upload_bytes(
        &context(),
        &digest_util().compute(&data),
        data,
        false,
    ))
    .unwrap_err();
    // assertThat(e.getCause()).isInstanceOf(TooLongFrameException.class)
    assert!(
        e.to_string()
            .starts_with("io.netty.handler.codec.TooLongFrameException"),
        "{e}"
    );
}
both_servers!(upload_response_too_large);

fn bar_handler() -> Handler {
    Arc::new(|_r: &Request| Answer::RespondAndClose(ok_with_body(b"bar")))
}

// Java: src/test/java/com/google/devtools/build/lib/remote/http/HttpCacheClientTest.java:527 HttpCacheClientTest.testDownloadFailsOnDigestMismatch
fn test_download_fails_on_digest_mismatch(kind: ServerKind) {
    // Test that the download fails when a blob/file has a different content hash than expected.
    let server = TestServer::start(kind, bar_handler());
    let blob_store = create_http_blob_store(
        &server,
        1,
        /* remote_verify_downloads= */ true,
        Vec::new(),
        Some(Arc::new(MockCredentials::default())),
        None,
    );
    let foo_digest = digest_util().compute(b"foo");
    let (result, _) = download(&blob_store, &foo_digest);
    let e = result.unwrap_err();
    assert!(e.to_string().contains(&foo_digest.hash), "{e}");
    assert!(
        e.to_string()
            .contains(&digest_util().compute_as_utf8("bar").hash),
        "{e}"
    );
}
both_servers!(test_download_fails_on_digest_mismatch);

// Java: src/test/java/com/google/devtools/build/lib/remote/http/HttpCacheClientTest.java:575 HttpCacheClientTest.testDisablingDigestVerification
fn test_disabling_digest_verification(kind: ServerKind) {
    // Test that when digest verification is disabled a corrupted download works.
    let server = TestServer::start(kind, bar_handler());
    let blob_store = create_http_blob_store(
        &server,
        1,
        /* remote_verify_downloads= */ false,
        Vec::new(),
        Some(Arc::new(MockCredentials::default())),
        None,
    );
    let foo_digest = digest_util().compute(b"foo");
    let (result, out) = download(&blob_store, &foo_digest);
    result.unwrap();
    assert_eq!(out, b"bar");
}
both_servers!(test_disabling_digest_verification);

/// `IntermittentFailureHandler`: the first request gets a chunked response cut after its first
/// chunk (the connection closes), later ones the full response.
fn intermittent_failure_handler(
    attempt1_chunk1: Vec<u8>,
    attempt2_chunk1: Vec<u8>,
    attempt2_chunk2: Vec<u8>,
) -> Handler {
    let message_count = Arc::new(AtomicUsize::new(0));
    Arc::new(move |_r: &Request| {
        let chunk = |data: &[u8]| {
            let mut c = format!("{:x}\r\n", data.len()).into_bytes();
            c.extend_from_slice(data);
            c.extend_from_slice(b"\r\n");
            c
        };
        let mut out = response("200 OK", &[("transfer-encoding", "chunked")], b"");
        if message_count.fetch_add(1, Ordering::SeqCst) == 0 {
            out.extend(chunk(&attempt1_chunk1));
        } else {
            out.extend(chunk(&attempt2_chunk1));
            out.extend(chunk(&attempt2_chunk2));
            out.extend_from_slice(b"0\r\n\r\n");
        }
        Answer::RespondAndClose(out)
    })
}

// Java: src/test/java/com/google/devtools/build/lib/remote/http/HttpCacheClientTest.java:617 HttpCacheClientTest.partialDownloadFailsWithoutRetry
fn partial_download_fails_without_retry(kind: ServerKind) {
    let server = TestServer::start(
        kind,
        intermittent_failure_handler(b"File ".to_vec(), b"File ".to_vec(), b"Contents".to_vec()),
    );
    let blob_store = simple_store(&server, Some(Arc::new(MockCredentials::default())));
    let (result, _) = download(&blob_store, &digest());
    let e = result.unwrap_err();
    assert!(
        errors::downcast_ref::<ClosedChannelException>(&e).is_some(),
        "{e:?}"
    );
}
both_servers!(partial_download_fails_without_retry);

// Java: src/test/java/com/google/devtools/build/lib/remote/http/HttpCacheClientTest.java:640 HttpCacheClientTest.partialDownloadSucceedsWithRetry
fn partial_download_succeeds_with_retry(kind: ServerKind) {
    // Replace first chunk to test that the client skips the redundant prefix on retry.
    let server = TestServer::start(
        kind,
        intermittent_failure_handler(b"File ".to_vec(), b"abcde".to_vec(), b"Contents".to_vec()),
    );
    let blob_store = create_http_blob_store(
        &server,
        1,
        /* remote_verify_downloads= */ false,
        Vec::new(),
        Some(Arc::new(MockCredentials::default())),
        Some(closed_channel_retrier()),
    );
    let (result, out) = download(&blob_store, &digest());
    result.unwrap();
    assert_eq!(out, b"File Contents");
}
both_servers!(partial_download_succeeds_with_retry);

// Java: src/test/java/com/google/devtools/build/lib/remote/http/HttpCacheClientTest.java:682 HttpCacheClientTest.actionResultRetryReadsFromStart
fn action_result_retry_reads_from_start(kind: ServerKind) {
    let action1 = ActionResult {
        output_files: vec![OutputFile {
            path: "attempt1/filename".to_string(),
            digest: Some(digest_util().compute_as_utf8("digest1")),
            is_executable: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    let buffer1 = action1.encode_to_vec();
    let split_at = buffer1.len() / 2;
    // Replace first chunk to test that the client starts a fresh ActionResult download on retry.
    let action2 = ActionResult {
        output_files: vec![OutputFile {
            path: "attempt2/filename".to_string(),
            digest: Some(digest_util().compute_as_utf8("digest2")),
            is_executable: false,
            ..Default::default()
        }],
        ..Default::default()
    };
    let buffer2 = action2.encode_to_vec();
    let server = TestServer::start(
        kind,
        intermittent_failure_handler(
            buffer1[..split_at].to_vec(),
            buffer2[..split_at].to_vec(),
            buffer2[split_at..].to_vec(),
        ),
    );
    let blob_store = create_http_blob_store(
        &server,
        1,
        /* remote_verify_downloads= */ false,
        Vec::new(),
        Some(Arc::new(MockCredentials::default())),
        Some(closed_channel_retrier()),
    );
    let action_result = get_from_future(blob_store.download_action_result(
        &context(),
        &ActionKey::new(digest()),
        false,
        &Default::default(),
    ))
    .unwrap();
    assert_eq!(action_result, Some(action2));
}
both_servers!(action_result_retry_reads_from_start);

/// `NotAuthorizedHandler.ErrorType`.
#[derive(Clone, Copy, Debug)]
enum ErrorType {
    Unauthorized,
    InvalidToken,
    InsufficientScope,
    InvalidRequest,
}

/// `NotAuthorizedHandler`: the first request (with "Bearer invalidToken") gets a 401 and the
/// connection closes; a second one (with "Bearer validToken") gets "File Contents".
fn not_authorized_handler(error_type: ErrorType) -> Handler {
    let message_count = Arc::new(AtomicUsize::new(0));
    Arc::new(move |request: &Request| {
        let internal_error = || {
            Answer::RespondAndClose(response(
                "500 Internal Server Error",
                &[("content-length", "0")],
                b"",
            ))
        };
        match message_count.load(Ordering::SeqCst) {
            0 => {
                if request.header("authorization") != Some("Bearer invalidToken") {
                    return internal_error();
                }
                message_count.fetch_add(1, Ordering::SeqCst);
                // DefaultFullHttpResponse without content: no Content-Length.
                let answer = match error_type {
                    ErrorType::Unauthorized => response("401 Unauthorized", &[], b""),
                    _ => {
                        let name = match error_type {
                            ErrorType::InvalidToken => "invalid_token",
                            ErrorType::InsufficientScope => "insufficient_scope",
                            ErrorType::InvalidRequest => "invalid_request",
                            ErrorType::Unauthorized => unreachable!(),
                        };
                        response(
                            "401 Unauthorized",
                            &[(
                                "www-authenticate",
                                &format!(
                                    "Bearer realm=\"localhost\",error=\"{name}\",error_description=\"The access token expired\""
                                ),
                            )],
                            b"",
                        )
                    }
                };
                Answer::RespondAndClose(answer)
            }
            1 => {
                if request.header("authorization") != Some("Bearer validToken") {
                    return internal_error();
                }
                message_count.fetch_add(1, Ordering::SeqCst);
                Answer::Respond(response(
                    "200 OK",
                    &[("connection", "keep-alive"), ("content-length", "13")],
                    b"File Contents",
                ))
            }
            // No third message expected.
            _ => internal_error(),
        }
    })
}

fn expired_auth_tokens_should_be_retried_get_with(kind: ServerKind, error_type: ErrorType) {
    let server = TestServer::start(kind, not_authorized_handler(error_type));
    let credentials = Arc::new(MockCredentials::default());
    let blob_store = simple_store(&server, Some(credentials.clone()));
    let (result, out) = download(&blob_store, &digest());
    result.unwrap();
    assert_eq!(out, b"File Contents");
    assert_eq!(credentials.refresh_count.load(Ordering::SeqCst), 1);
    assert_eq!(
        credentials
            .get_request_metadata_count
            .load(Ordering::SeqCst),
        2
    );
}

// Java: src/test/java/com/google/devtools/build/lib/remote/http/HttpCacheClientTest.java:750 HttpCacheClientTest.expiredAuthTokensShouldBeRetried_get
fn expired_auth_tokens_should_be_retried_get(kind: ServerKind) {
    expired_auth_tokens_should_be_retried_get_with(kind, ErrorType::Unauthorized);
    expired_auth_tokens_should_be_retried_get_with(kind, ErrorType::InvalidToken);
}
both_servers!(expired_auth_tokens_should_be_retried_get);

fn expired_auth_tokens_should_be_retried_put_with(kind: ServerKind, error_type: ErrorType) {
    let server = TestServer::start(kind, not_authorized_handler(error_type));
    let credentials = Arc::new(MockCredentials::default());
    let blob_store = simple_store(&server, Some(credentials.clone()));
    let data = bytes::Bytes::from_static(b"File Contents");
    get_from_future(blob_store.upload_bytes(
        &context(),
        &digest_util().compute(&data),
        data,
        false,
    ))
    .unwrap();
    assert_eq!(credentials.refresh_count.load(Ordering::SeqCst), 1);
    assert_eq!(
        credentials
            .get_request_metadata_count
            .load(Ordering::SeqCst),
        2
    );
}

// Java: src/test/java/com/google/devtools/build/lib/remote/http/HttpCacheClientTest.java:782 HttpCacheClientTest.expiredAuthTokensShouldBeRetried_put
fn expired_auth_tokens_should_be_retried_put(kind: ServerKind) {
    expired_auth_tokens_should_be_retried_put_with(kind, ErrorType::Unauthorized);
    expired_auth_tokens_should_be_retried_put_with(kind, ErrorType::InvalidToken);
}
both_servers!(expired_auth_tokens_should_be_retried_put);

fn assert_unauthorized(e: &io::Error) {
    let http_exception = errors::downcast_ref::<HttpException>(e)
        .unwrap_or_else(|| panic!("an HttpException: {e:?}"));
    assert_eq!(http_exception.status, 401);
}

// Java: src/test/java/com/google/devtools/build/lib/remote/http/HttpCacheClientTest.java:817 HttpCacheClientTest.errorCodesThatShouldNotBeRetried_get
fn error_codes_that_should_not_be_retried_get(kind: ServerKind) {
    for error_type in [ErrorType::InsufficientScope, ErrorType::InvalidRequest] {
        let server = TestServer::start(kind, not_authorized_handler(error_type));
        let blob_store = simple_store(&server, Some(Arc::new(MockCredentials::default())));
        let (result, _) = download(&blob_store, &digest());
        assert_unauthorized(&result.unwrap_err());
    }
}
both_servers!(error_codes_that_should_not_be_retried_get);

// Java: src/test/java/com/google/devtools/build/lib/remote/http/HttpCacheClientTest.java:848 HttpCacheClientTest.errorCodesThatShouldNotBeRetried_put
fn error_codes_that_should_not_be_retried_put(kind: ServerKind) {
    for error_type in [ErrorType::InsufficientScope, ErrorType::InvalidRequest] {
        let server = TestServer::start(kind, not_authorized_handler(error_type));
        let blob_store = simple_store(&server, Some(Arc::new(MockCredentials::default())));
        let one_byte = bytes::Bytes::from_static(&[0]);
        let e = get_from_future(blob_store.upload_bytes(
            &context(),
            &digest_util().compute(&one_byte),
            one_byte,
            false,
        ))
        .unwrap_err();
        assert_unauthorized(&e);
    }
}
both_servers!(error_codes_that_should_not_be_retried_put);

// Java: src/test/java/com/google/devtools/build/lib/remote/http/HttpCacheClientTest.java:1017 HttpCacheClientTest.extraCacheHeaders
fn extra_cache_headers(kind: ServerKind) {
    let mut remote_options = blaze_options::Options::<
        blaze_optdefs::misc::remote_options::RemoteOptions,
    >::get_defaults();
    remote_options.remote_headers = vec![
        ("CommonKey1".into(), "CommonValue1".into()),
        ("CommonKey2".into(), "CommonValue2".into()),
    ];
    remote_options.remote_cache_headers = vec![
        ("CacheKey1".into(), "CacheValue1".into()),
        ("CacheKey2".into(), "CacheValue2".into()),
    ];
    remote_options.remote_exec_headers = vec![
        ("ExecKey1".into(), "ExecValue1".into()),
        ("ExecKey2".into(), "ExecValue2".into()),
    ];
    let seen: Arc<Mutex<Vec<Request>>> = Arc::new(Mutex::new(Vec::new()));
    let seen_by_server = seen.clone();
    let server = TestServer::start(
        kind,
        Arc::new(move |request: &Request| {
            seen_by_server.lock().unwrap().push(request.clone());
            Answer::RespondAndClose(ok_with_body(b"File Contents"))
        }),
    );
    let blob_store = create_http_blob_store(
        &server,
        1,
        true,
        blaze_remote::combined_cache_client_factory::effective_headers(&remote_options),
        Some(Arc::new(MockCredentials::default())),
        None,
    );
    let (result, out) = download(&blob_store, &digest());
    result.unwrap();
    assert_eq!(out, b"File Contents");
    let seen = seen.lock().unwrap();
    let request = &seen[0];
    assert_eq!(request.header("commonkey1"), Some("CommonValue1"));
    assert_eq!(request.header("commonkey2"), Some("CommonValue2"));
    assert_eq!(request.header("cachekey1"), Some("CacheValue1"));
    assert_eq!(request.header("cachekey2"), Some("CacheValue2"));
    assert_eq!(request.header("execkey1"), None);
    assert_eq!(request.header("execkey2"), None);
}
both_servers!(extra_cache_headers);

/// Not in the Java test: a 404 is a `CacheNotFoundException` (`cacheMiss`), and the request
/// heads are the oracle's (recorded with `scripts/remote-http/log-server`: lower-case names, in
/// Netty's order).
#[test]
fn cache_miss_and_request_heads() {
    let seen: Arc<Mutex<Vec<Request>>> = Arc::new(Mutex::new(Vec::new()));
    let seen_by_server = seen.clone();
    let contents = Arc::new(Mutex::new(HashMap::new()));
    let cache = in_memory_cache(contents);
    let server = TestServer::start(
        ServerKind::Inet,
        Arc::new(move |request: &Request| {
            seen_by_server.lock().unwrap().push(request.clone());
            cache(request)
        }),
    );
    let blob_store = simple_store(&server, None);
    let (result, _) = download(&blob_store, &digest());
    let e = result.unwrap_err();
    assert!(
        errors::downcast_ref::<CacheNotFoundException>(&e).is_some(),
        "{e:?}"
    );
    let data = bytes::Bytes::from_static(b"foo bar");
    get_from_future(blob_store.upload_bytes(
        &context(),
        &digest_util().compute(&data),
        data,
        false,
    ))
    .unwrap();
    let seen = seen.lock().unwrap();
    let port = server.port.unwrap();
    let names = |r: &Request| r.headers.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>();
    assert_eq!(
        names(&seen[0]),
        [
            "host",
            "connection",
            "accept",
            "accept-encoding",
            "user-agent"
        ]
    );
    assert_eq!(seen[0].header("host"), Some(&*format!("localhost:{port}")));
    assert_eq!(
        seen[0].header("accept-encoding"),
        Some("gzip,deflate,snappy,zstd")
    );
    assert_eq!(seen[0].header("user-agent"), Some("bazel/9.3.2"));
    assert_eq!(
        names(&seen[1]),
        [
            "host",
            "accept",
            "content-length",
            "connection",
            "user-agent"
        ]
    );
    // The connection was kept alive and reused: both requests came on one connection is not
    // observable here; the second request was answered.
    assert_eq!(seen.len(), 2);
}

/// Not in the Java test (review 2026-11-19 round 8, H1): a server that answers before it has
/// read the body. Java's `HttpUploadHandler.channelRead0` takes the response while the body is
/// still being written; the client must report the status at once, not wait for the body to be
/// written (an upload timeout once the socket buffers are full).
const LARGE_BODY: usize = 16 * 1024 * 1024;

fn large_put_answered_early(
    answer: impl Fn(&Request) -> Option<Answer> + Send + Sync + 'static,
) -> EarlyHandler {
    Arc::new(move |request: &Request| {
        let length: usize = request
            .header("content-length")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        if request.method == "PUT" && length > 1024 * 1024 {
            answer(request)
        } else {
            None
        }
    })
}

fn ok_handler() -> Handler {
    Arc::new(|_request: &Request| {
        Answer::Respond(response(
            "200 OK",
            &[("connection", "keep-alive"), ("content-length", "0")],
            b"",
        ))
    })
}

fn upload_answered_before_body_reports_the_status(kind: ServerKind) {
    let server = TestServer::start_with_early(
        kind,
        ok_handler(),
        Some(large_put_answered_early(|_| {
            Some(Answer::Respond(response(
                "403 Forbidden",
                &[("content-length", "0")],
                b"",
            )))
        })),
    );
    let blob_store = create_http_blob_store(&server, 10, true, Vec::new(), None, None);
    let data = bytes::Bytes::from(vec![7u8; LARGE_BODY]);
    let start = std::time::Instant::now();
    let e = get_from_future(blob_store.upload_bytes(
        &context(),
        &digest_util().compute(&data),
        data,
        false,
    ))
    .unwrap_err();
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "{:?}",
        start.elapsed()
    );
    let http_exception = errors::downcast_ref::<HttpException>(&e).expect("an HttpException");
    assert_eq!(http_exception.status, 403);
    assert_eq!(e.to_string(), "403 Forbidden");
}
both_servers!(upload_answered_before_body_reports_the_status);

/// A 401 for an expired token before the body is read: refreshed and uploaded again, for a blob
/// whose stream Java can reset (a `ByteString`'s). The answered connection goes back to the pool
/// with the body unwritten (`HttpUploadHandler.channelRead0`, `releaseUploadChannel`) and the
/// retry takes it; this server keeps it open without reading, so the retry times out
/// (`UploadTimeoutException`), as Java's does (review 2026-11-19 round 9, J1; the oracle's
/// `authput` probe: "Upload of '/cas/…' timed out").
fn expired_auth_token_before_body_is_refreshed(kind: ServerKind) {
    let server = TestServer::start_with_early(
        kind,
        ok_handler(),
        Some(large_put_answered_early(|request| {
            (request.header("authorization") != Some("Bearer validToken")).then(|| {
                Answer::Respond(response(
                    "401 Unauthorized",
                    &[
                        (
                            "www-authenticate",
                            "Bearer realm=\"localhost\",error=\"invalid_token\"",
                        ),
                        ("content-length", "0"),
                    ],
                    b"",
                ))
            })
        })),
    );
    let credentials = Arc::new(MockCredentials::default());
    let blob_store = create_http_blob_store(
        &server,
        1,
        true,
        Vec::new(),
        Some(credentials.clone()),
        None,
    );
    let data = bytes::Bytes::from(vec![7u8; LARGE_BODY]);
    let e = get_from_future(blob_store.upload_bytes(
        &context(),
        &digest_util().compute(&data),
        data,
        false,
    ))
    .unwrap_err();
    assert_eq!(credentials.refresh_count.load(Ordering::SeqCst), 1);
    assert!(
        errors::find_cause::<UploadTimeoutException>(&e).is_some()
            || errors::downcast_ref::<UploadTimeoutException>(&e).is_some(),
        "{e:?}"
    );
}
both_servers!(expired_auth_token_before_body_is_refreshed);

/// Review round 8, S4: the upload of a file is not repeated after a refresh, as Java's
/// `reset(in)` fails for a `LazyFileInputStream`; the 401 is the error.
fn expired_auth_token_for_a_file_is_not_retried(kind: ServerKind) {
    let server = TestServer::start_with_early(
        kind,
        ok_handler(),
        Some(large_put_answered_early(|_| {
            Some(Answer::Respond(response(
                "401 Unauthorized",
                &[
                    (
                        "www-authenticate",
                        "Bearer realm=\"localhost\",error=\"invalid_token\"",
                    ),
                    ("content-length", "0"),
                ],
                b"",
            )))
        })),
    );
    let credentials = Arc::new(MockCredentials::default());
    let blob_store = create_http_blob_store(
        &server,
        10,
        true,
        Vec::new(),
        Some(credentials.clone()),
        None,
    );
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("big");
    let data = vec![7u8; LARGE_BODY];
    std::fs::write(&file_path, &data).unwrap();
    let fs: blaze_vfs::FileSystemRef =
        Arc::new(blaze_vfs::UnixFileSystem::new(DigestHashFunction::Sha256));
    let file = blaze_vfs::Path::create_from_str(file_path.to_str().unwrap(), &fs);
    let e = get_from_future(blob_store.upload_file(
        &context(),
        &digest_util().compute(&data),
        &file,
        false,
    ))
    .unwrap_err();
    let http_exception = errors::downcast_ref::<HttpException>(&e).expect("an HttpException");
    assert_eq!(http_exception.status, 401);
    assert_eq!(credentials.refresh_count.load(Ordering::SeqCst), 0);
}
both_servers!(expired_auth_token_for_a_file_is_not_retried);

/// `createHttpBlobStore` with `--remote_max_connections`.
fn store_with_max_connections(
    server: &TestServer,
    remote_max_connections: i32,
) -> Arc<HttpCacheClient> {
    let (socket, uri, tls) = server_address(server);
    Arc::new(
        HttpCacheClient::create(
            socket,
            JavaUri::create(&uri).unwrap(),
            /* timeout_seconds= */ 5,
            remote_max_connections,
            /* verify_downloads= */ true,
            Vec::new(),
            digest_util(),
            default_retrier(),
            None,
            tls,
            "9.3.2",
        )
        .unwrap(),
    )
}

/// Review 2026-11-19 round 9, J1: `uploadAsync` acquires its channel on the calling thread, so
/// uploads started together take their channels in call order; with one connection
/// (`FixedChannelPool` of 1) the server reads them in that order (the queued acquisitions are
/// served first in first out).
fn uploads_take_channels_in_call_order(kind: ServerKind) {
    let order = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen = order.clone();
    let server = TestServer::start(
        kind,
        Arc::new(move |request: &Request| {
            seen.lock().unwrap().push(request.path.clone());
            Answer::Respond(response("200 OK", &[("content-length", "0")], b""))
        }),
    );
    let store = store_with_max_connections(&server, 1);
    let blobs: Vec<bytes::Bytes> = (0..8)
        .map(|i| bytes::Bytes::from(format!("blob {i}").repeat(1000 * (8 - i))))
        .collect();
    let digests: Vec<Digest> = blobs.iter().map(|b| digest_util().compute(b)).collect();
    let context = context();
    let uploads: Vec<_> = blobs
        .iter()
        .zip(&digests)
        .map(|(blob, digest)| store.upload_bytes(&context, digest, blob.clone(), false))
        .collect();
    let results =
        get_from_future(async { Ok::<_, io::Error>(futures::future::join_all(uploads).await) })
            .unwrap();
    for result in results {
        result.unwrap();
    }
    let expected: Vec<String> = digests.iter().map(|d| format!("/cas/{}", d.hash)).collect();
    assert_eq!(*order.lock().unwrap(), expected);
}
both_servers!(uploads_take_channels_in_call_order);

/// Downloads a caller waits for together proceed at once, each on its own channel, as in Java
/// (`get` acquires a channel per download): the server answers neither until both requests have
/// arrived.
fn downloads_proceed_together(kind: ServerKind) {
    let foo = b"foo".to_vec();
    let bar = b"bar".to_vec();
    let mut contents = HashMap::new();
    contents.insert(
        format!("/cas/{}", digest_util().compute(&foo[..]).hash),
        foo.clone(),
    );
    contents.insert(
        format!("/cas/{}", digest_util().compute(&bar[..]).hash),
        bar.clone(),
    );
    let arrived = Arc::new(AtomicUsize::new(0));
    let server = TestServer::start(
        kind,
        Arc::new(move |request: &Request| {
            arrived.fetch_add(1, Ordering::SeqCst);
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            while arrived.load(Ordering::SeqCst) < 2 && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            if arrived.load(Ordering::SeqCst) < 2 {
                return Answer::Respond(response("404 Not Found", &[("content-length", "0")], b""));
            }
            Answer::Respond(ok_with_body(&contents[&request.path]))
        }),
    );
    let store = store_with_max_connections(&server, 0);
    let context = context();
    let foo_digest = digest_util().compute(&foo[..]);
    let bar_digest = digest_util().compute(&bar[..]);
    let mut foo_out: Vec<u8> = Vec::new();
    let mut bar_out: Vec<u8> = Vec::new();
    let (a, b) = get_from_future(async {
        Ok::<_, io::Error>(
            futures::future::join(
                store.download_blob(&context, &foo_digest, &mut foo_out),
                store.download_blob(&context, &bar_digest, &mut bar_out),
            )
            .await,
        )
    })
    .unwrap();
    a.unwrap();
    b.unwrap();
    assert_eq!(foo_out, foo);
    assert_eq!(bar_out, bar);
}
both_servers!(downloads_proceed_together);

/// Review 2026-11-19 round 9, S3: over TLS, an answer that arrives while the client still holds
/// unsent records is read at once (`tls_read` decrypts what has arrived without first flushing
/// those records, as Netty's `SslHandler` decodes inbound records independently of its pending
/// writes). The server reads the head, then nothing, and answers after half a second: by then
/// the socket buffers are full (the body is larger than they grow) and rustls holds records it
/// could not send. Before the fix the
/// read first blocked in a write to the server that does not read, until the server closed the
/// connection (5 s), and the status was lost.
#[test]
fn upload_answered_before_body_reports_the_status_over_tls() {
    let server = TestServer::start_with_early(
        ServerKind::Tls,
        ok_handler(),
        Some(large_put_answered_early(|_| {
            std::thread::sleep(Duration::from_millis(500));
            Some(Answer::Respond(response(
                "403 Forbidden",
                &[("content-length", "0")],
                b"",
            )))
        })),
    );
    let blob_store = create_http_blob_store(&server, 10, true, Vec::new(), None, None);
    // More than the loopback socket buffers take (`tcp_rmem` grows to 32 MiB here).
    let data = bytes::Bytes::from(vec![7u8; 96 * 1024 * 1024]);
    let start = std::time::Instant::now();
    let e = get_from_future(blob_store.upload_bytes(
        &context(),
        &digest_util().compute(&data),
        data,
        false,
    ))
    .unwrap_err();
    assert!(
        start.elapsed() < Duration::from_secs(4),
        "{:?}",
        start.elapsed()
    );
    let http_exception = errors::downcast_ref::<HttpException>(&e).expect("an HttpException");
    assert_eq!(http_exception.status, 403);
}

/// The TLS test server itself: a plain upload and download over TLS.
#[test]
fn test_upload_over_tls() {
    test_upload(ServerKind::Tls);
}
