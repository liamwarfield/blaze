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
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

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

#[derive(Clone, Copy, Debug)]
enum ServerKind {
    Inet,
    Unix,
}

struct TestServer {
    port: Option<u16>,
    socket: Option<PathBuf>,
}

trait Connection: Read + Write + Send {}
impl Connection for TcpStream {}
impl Connection for UnixStream {}

fn read_request(reader: &mut BufReader<Box<dyn Connection>>) -> Option<Request> {
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
    let length: usize = headers
        .iter()
        .find(|(k, _)| k == "content-length")
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).ok()?;
    Some(Request {
        method,
        path,
        headers,
        body,
    })
}

fn serve(stream: Box<dyn Connection>, handler: Handler) {
    let mut reader = BufReader::new(stream);
    while let Some(request) = read_request(&mut reader) {
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
        match kind {
            ServerKind::Inet => {
                let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
                let port = listener.local_addr().unwrap().port();
                std::thread::spawn(move || {
                    for stream in listener.incoming() {
                        let Ok(stream) = stream else { return };
                        let handler = handler.clone();
                        std::thread::spawn(move || serve(Box::new(stream), handler));
                    }
                });
                TestServer {
                    port: Some(port),
                    socket: None,
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
                        std::thread::spawn(move || serve(Box::new(stream), handler));
                    }
                });
                TestServer {
                    port: None,
                    socket: Some(path),
                }
            }
        }
    }

    /// A server whose socket is already gone (`testServer.stop(server)` before the client
    /// connects).
    fn stopped(kind: ServerKind) -> TestServer {
        match kind {
            ServerKind::Inet => {
                let port = TcpListener::bind(("127.0.0.1", 0))
                    .unwrap()
                    .local_addr()
                    .unwrap()
                    .port();
                TestServer {
                    port: Some(port),
                    socket: None,
                }
            }
            ServerKind::Unix => TestServer {
                port: None,
                socket: Some(std::env::temp_dir().join(format!(
                    "blaze-http-cache-test-{}-stopped.sock",
                    std::process::id()
                ))),
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
    let (socket, uri) = match (&server.socket, server.port) {
        (Some(socket), _) => (Some(socket.clone()), "http://localhost".to_string()),
        (None, Some(port)) => (None, format!("http://localhost:{port}")),
        (None, None) => unreachable!(),
    };
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
            /* tls= */ None,
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
