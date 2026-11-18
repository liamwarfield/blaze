//! Tests for the registry fetcher seam (`DownloadManagerRegistryFetcher` over
//! `blaze_repo_fetch`'s `DownloadManager`): ports of the `HttpDownloaderTest` cases on the
//! `downloadAndRead` / `downloadAndReadOneUrlForBzlmod` path, a few `HttpConnectorTest` cases,
//! and checks of the repository-cache behaviour of
//! `DownloadManager.downloadAndReadOneUrlForBzlmod`, against a local HTTP server on 127.0.0.1.
//!
//! `DownloadCacheTest` is ported in `crates/blaze_repo_fetch/tests/download_cache_test.rs` (the
//! in-memory slice of the cache that this file used to test was the deleted stand-in's).

use std::any::Any;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use blaze_bzlmod::registry_fetcher::{CAS_DIR, DownloadManagerRegistryFetcher};
use blaze_bzlmod_model::registry::{RegistryException, RegistryFetcher};
use blaze_bzlmod_model::{Checksum, JavaUri, KeyType};
use blaze_events::{Event, EventHandler, ExtendedEventHandler, Postable};
use blaze_repo_fetch::cache::DownloadCacheHitEvent;
use blaze_repo_fetch::cache::download_cache::DEFAULT_CACHE_FILENAME;
use blaze_repo_fetch::downloader::HttpDownloader;
use indexmap::IndexMap;

// ---------------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------------

#[derive(Default)]
struct RecordingHandler {
    events: Mutex<Vec<String>>,
    posts: Mutex<Vec<Arc<dyn Postable>>>,
}

impl RecordingHandler {
    fn cache_hits(&self) -> Vec<DownloadCacheHitEvent> {
        self.posts
            .lock()
            .unwrap()
            .iter()
            .filter_map(|p| {
                let any: &dyn Any = p.as_ref();
                any.downcast_ref::<DownloadCacheHitEvent>().cloned()
            })
            .collect()
    }
}

impl EventHandler for RecordingHandler {
    fn handle(&self, event: &Event) {
        self.events
            .lock()
            .unwrap()
            .push(event.get_message().into_owned());
    }

    fn as_extended_event_handler(&self) -> Option<&dyn ExtendedEventHandler> {
        Some(self)
    }
}

impl ExtendedEventHandler for RecordingHandler {
    fn post(&self, obj: Arc<dyn Postable>) {
        self.posts.lock().unwrap().push(obj);
    }
}

/// A server on 127.0.0.1 that answers one connection per entry of `responses`, in order, and
/// records each request head.
struct Server {
    port: u16,
    requests: Arc<Mutex<Vec<String>>>,
    handle: Option<JoinHandle<()>>,
}

impl Server {
    fn start(responses: Vec<Vec<u8>>) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        let handle = std::thread::spawn(move || {
            for response in responses {
                let (mut socket, _) = listener.accept().unwrap();
                recorded
                    .lock()
                    .unwrap()
                    .push(read_http_request(&mut socket));
                let _ = socket.write_all(&response);
                let _ = socket.flush();
                let _ = socket.shutdown(std::net::Shutdown::Both);
            }
        });
        Server {
            port,
            requests,
            handle: Some(handle),
        }
    }

    fn url(&self, path: &str) -> JavaUri {
        JavaUri::new(&format!("http://127.0.0.1:{}{}", self.port, path)).unwrap()
    }

    fn requests(&mut self) -> Vec<String> {
        if let Some(h) = self.handle.take() {
            h.join().unwrap();
        }
        self.requests.lock().unwrap().clone()
    }
}

/// `DownloaderTestUtils.readHttpRequest`: reads the request head.
fn read_http_request(socket: &mut TcpStream) -> String {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        match socket.read(&mut byte) {
            Ok(0) | Err(_) => break,
            Ok(_) => head.push(byte[0]),
        }
    }
    String::from_utf8_lossy(&head).into_owned()
}

/// `DownloaderTestUtils.sendLines`: the lines joined with CRLF.
fn lines(data: &[&str]) -> Vec<u8> {
    data.join("\r\n").into_bytes()
}

fn ok_response(body: &str) -> Vec<u8> {
    let len = body.len().to_string();
    lines(&[
        "HTTP/1.1 200 OK",
        "Date: Fri, 31 Dec 1999 23:59:59 GMT",
        "Connection: close",
        "Content-Type: text/plain",
        &format!("Content-Length: {len}"),
        "",
        body,
    ])
}

fn not_found_response() -> Vec<u8> {
    lines(&[
        "HTTP/1.1 404 Not Found",
        "Date: Fri, 31 Dec 1999 23:59:59 GMT",
        "Connection: close",
        "Content-Type: text/plain",
        "Content-Length: 0",
        "",
        "",
    ])
}

fn sha256_hex(bytes: &[u8]) -> String {
    Checksum::sha256_of(bytes).to_string()
}

fn sha256(bytes: &[u8]) -> Checksum {
    Checksum::from_string(KeyType::Sha256, &sha256_hex(bytes)).unwrap()
}

/// A fetcher with `HttpDownloaderTest`'s downloader (`new HttpDownloader(0, Duration.ZERO, 8,
/// .1f)`: timeouts scaled down to make tests fast).
fn fetcher(
    repository_cache: Option<PathBuf>,
    retries: i32,
) -> (DownloadManagerRegistryFetcher, Arc<RecordingHandler>) {
    let handler = Arc::new(RecordingHandler::default());
    let f = DownloadManagerRegistryFetcher::create(
        repository_cache.as_deref(),
        test_http_downloader(),
        handler.clone(),
        retries,
    );
    (f, handler)
}

fn test_http_downloader() -> HttpDownloader {
    HttpDownloader::new(0, Duration::ZERO, 8, 0.1)
}

fn rf_uri(uri: &JavaUri) -> blaze_repo_fetch::uri::JavaUri {
    blaze_repo_fetch::uri::JavaUri::create(&uri.to_string()).unwrap()
}

fn no_env() -> IndexMap<String, String> {
    IndexMap::new()
}

fn cache_file(repository_cache: &Path, hex: &str) -> PathBuf {
    repository_cache
        .join(CAS_DIR)
        .join("sha256")
        .join(hex)
        .join(DEFAULT_CACHE_FILENAME)
}

// ---------------------------------------------------------------------------------------------
// HttpDownloaderTest (downloadAndRead / downloadAndReadOneUrlForBzlmod)
// ---------------------------------------------------------------------------------------------

// Java: src/test/java/com/google/devtools/build/lib/bazel/repository/downloader/HttpDownloaderTest.java:763 HttpDownloaderTest.downloadAndRead_ok
#[test]
fn download_and_read_ok() {
    let mut server = Server::start(vec![ok_response("hello")]);
    let (f, _) = fetcher(None, 0);
    let content = f
        .download_and_read_one_url_for_bzlmod(&server.url("/foo"), &no_env(), None)
        .unwrap();
    assert_eq!(content, b"hello");
    assert_eq!(server.requests().len(), 1);
}

// Java: src/test/java/com/google/devtools/build/lib/bazel/repository/downloader/HttpDownloaderTest.java:800 HttpDownloaderTest.downloadAndRead_notFound
#[test]
fn download_and_read_not_found() {
    let mut server = Server::start(vec![lines(&[
        "HTTP/1.1 404 Not Found",
        "Date: Fri, 31 Dec 1999 23:59:59 GMT",
        "Connection: close",
        "Content-Type: text/plain",
        "Content-Length: 5",
        "",
        "",
    ])]);
    let (f, _) = fetcher(None, 0);
    let e = f
        .download_and_read_one_url_for_bzlmod(&server.url("/foo"), &no_env(), None)
        .unwrap_err();
    // HttpConnector throws FileNotFoundException(describeHttpResponse(connection)).
    assert_eq!(
        e,
        RegistryException::NotFound("GET returned 404 Not Found".to_string())
    );
    // Permanent errors are not retried.
    assert_eq!(server.requests().len(), 1);
}

// Java: src/test/java/com/google/devtools/build/lib/bazel/repository/downloader/HttpDownloaderTest.java:835 HttpDownloaderTest.downloadAndRead_checksumProvided
#[test]
fn download_and_read_checksum_provided() {
    let server = Server::start(vec![ok_response("hello")]);
    let url = server.url("/foo");
    let (f, _) = fetcher(None, 0);
    let content = f
        .download_and_read_one_url_for_bzlmod(&url, &no_env(), Some(&sha256(b"hello")))
        .unwrap();
    assert_eq!(content, b"hello");
}

// Java: src/test/java/com/google/devtools/build/lib/bazel/repository/downloader/HttpDownloaderTest.java:876 HttpDownloaderTest.downloadAndRead_checksumMismatch
#[test]
fn download_and_read_checksum_mismatch() {
    let server = Server::start(vec![lines(&[
        "HTTP/1.1 200 OK",
        "Date: Fri, 31 Dec 1999 23:59:59 GMT",
        "Connection: close",
        "Content-Type: text/plain",
        "Content-Length: 9",
        "",
        "malicious",
    ])]);
    // Hashing.sha256().hashUnencodedChars("hello"): the UTF-16LE code units.
    let wanted = sha256(b"h\0e\0l\0l\0o\0");
    let (f, _) = fetcher(None, 0);
    let e = f
        .download_and_read_one_url_for_bzlmod(&server.url("/foo"), &no_env(), Some(&wanted))
        .unwrap_err();
    assert_eq!(
        e,
        RegistryException::Io(format!(
            "Checksum was {} but wanted {}",
            sha256_hex(b"malicious"),
            wanted
        ))
    );
}

// Java: src/test/java/com/google/devtools/build/lib/bazel/repository/downloader/HttpDownloaderTest.java:917 HttpDownloaderTest.downloadAndRead_twoUrlsFirstNotFoundAndSecondOk
#[test]
fn download_and_read_two_urls_first_not_found_and_second_ok() {
    let mut server1 = Server::start(vec![not_found_response()]);
    let mut server2 = Server::start(vec![lines(&[
        "HTTP/1.1 200 OK",
        "Date: Fri, 31 Dec 1999 23:59:59 GMT",
        "Connection: close",
        "Content-Type: text/plain",
        "",
        "content2",
    ])]);
    let handler: Arc<dyn ExtendedEventHandler> = Arc::new(RecordingHandler::default());
    let content = test_http_downloader()
        .download_and_read(
            &[rf_uri(&server1.url("/foo")), rf_uri(&server2.url("/foo"))],
            Arc::new(blaze_repo_fetch::authandtls::StaticCredentials::empty()),
            None,
            handler,
            &std::collections::HashMap::new(),
        )
        .unwrap();
    assert_eq!(content, b"content2");
    assert_eq!(server1.requests().len(), 1);
    assert_eq!(server2.requests().len(), 1);
}

// Java: src/test/java/com/google/devtools/build/lib/bazel/repository/downloader/HttpDownloaderTest.java:978 HttpDownloaderTest.downloadAndReadOneUrlForBzlmod_fallsBackToNextRewrittenUrl
// PORT: Java mocks `UrlRewriter.amend`; here a real `--downloader_config` expands the registry
// URL into the ordered [primary, fallback] list.
#[test]
fn download_and_read_one_url_for_bzlmod_falls_back_to_next_rewritten_url() {
    // The first (e.g. proxy/mirror) URL fails ...
    let mut primary = Server::start(vec![lines(&[
        "HTTP/1.1 404 Not Found",
        "Date: Fri, 31 Dec 1999 23:59:59 GMT",
        "Connection: close",
        "Content-Type: text/plain",
        "Content-Length: 0",
        "",
        "",
    ])]);
    // ... so the read must fall back to the second (e.g. direct) URL.
    let mut fallback = Server::start(vec![lines(&[
        "HTTP/1.1 200 OK",
        "Date: Fri, 31 Dec 1999 23:59:59 GMT",
        "Connection: close",
        "Content-Type: text/plain",
        "",
        "content2",
    ])]);
    let original_url = JavaUri::new("https://example.com/foo").unwrap();
    let config = format!(
        "rewrite example.com/foo http://127.0.0.1:{}/foo\n\
         rewrite example.com/foo http://127.0.0.1:{}/foo\n",
        primary.port, fallback.port
    );
    let rewriter =
        blaze_repo_fetch::downloader::UrlRewriter::new(&["downloader.cfg".to_string()], &[config])
            .unwrap();

    let handler = Arc::new(RecordingHandler::default());
    let http_downloader = Arc::new(test_http_downloader());
    let mut download_manager = blaze_repo_fetch::downloader::DownloadManager::new(
        blaze_repo_fetch::cache::RepositoryCache::new()
            .get_download_cache()
            .clone(),
        Arc::new(blaze_repo_fetch::downloader::DelegatingDownloader::new(
            http_downloader.clone(),
        )),
        http_downloader,
        handler,
    );
    download_manager.set_url_rewriter(Some(rewriter));
    let f = DownloadManagerRegistryFetcher::new(Arc::new(download_manager));

    let content = f
        .download_and_read_one_url_for_bzlmod(&original_url, &no_env(), None)
        .unwrap();

    assert_eq!(content, b"content2");
    assert_eq!(primary.requests().len(), 1);
    assert_eq!(fallback.requests().len(), 1);
}

// Java: src/test/java/com/google/devtools/build/lib/bazel/repository/downloader/HttpDownloaderTest.java:1048 HttpDownloaderTest.download_contentLengthMismatch_propagateErrorIfNotRetry
// PORT: Java mocks HttpDownloader.download; here the bzlmod path is driven by a real server whose
// body is shorter than its Content-Length.
#[test]
fn download_content_length_mismatch_propagate_error_if_not_retry() {
    let truncated = lines(&[
        "HTTP/1.1 200 OK",
        "Connection: close",
        "Content-Length: 10",
        "",
        "short",
    ]);
    let mut server = Server::start(vec![truncated]);
    let (f, _) = fetcher(None, 0);
    let e = f
        .download_and_read_one_url_for_bzlmod(&server.url("/foo"), &no_env(), None)
        .unwrap_err();
    assert_eq!(
        e,
        RegistryException::Io("Bytes read 5 but wanted 10".to_string())
    );
    assert_eq!(server.requests().len(), 1);
}

// Java: src/test/java/com/google/devtools/build/lib/bazel/repository/downloader/HttpDownloaderTest.java:1085 HttpDownloaderTest.download_contentLengthMismatch_retries
#[test]
fn download_content_length_mismatch_retries() {
    let truncated = lines(&[
        "HTTP/1.1 200 OK",
        "Connection: close",
        "Content-Length: 10",
        "",
        "short",
    ]);
    let mut server = Server::start(vec![truncated.clone(), truncated, ok_response("content")]);
    let (f, _) = fetcher(None, 5);
    let content = f
        .download_and_read_one_url_for_bzlmod(&server.url("/foo"), &no_env(), None)
        .unwrap();
    assert_eq!(content, b"content");
    assert_eq!(server.requests().len(), 3);
}

// A connection the server closes without answering is retried (HttpConnector retries every
// IOException that is not a permanent HTTP error).
#[test]
fn connection_closed_without_response_is_retried() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        // First connection: read the request and close without a response.
        let (mut socket, _) = listener.accept().unwrap();
        read_http_request(&mut socket);
        drop(socket);
        // Then answer normally.
        let (mut socket, _) = listener.accept().unwrap();
        read_http_request(&mut socket);
        socket.write_all(&ok_response("content")).unwrap();
    });
    let (f, _) = fetcher(None, 5);
    let url = JavaUri::new(&format!("http://127.0.0.1:{port}/foo")).unwrap();
    let content = f
        .download_and_read_one_url_for_bzlmod(&url, &no_env(), None)
        .unwrap();
    assert_eq!(content, b"content");
    handle.join().unwrap();
}

// ---------------------------------------------------------------------------------------------
// HttpConnectorTest (through the downloader)
// ---------------------------------------------------------------------------------------------

// Java: src/test/java/com/google/devtools/build/lib/bazel/repository/downloader/HttpConnectorTest.java:125 HttpConnectorTest.normalRequest
#[test]
fn normal_request() {
    let mut server = Server::start(vec![ok_response("hello")]);
    let (f, _) = fetcher(None, 0);
    f.download_and_read_one_url_for_bzlmod(&server.url("/boo"), &no_env(), None)
        .unwrap();
    let requests = server.requests();
    let head = requests[0].to_ascii_lowercase();
    assert!(head.starts_with("get /boo http/1.1\r\n"), "{head}");
    // HttpConnectorMultiplexer.REQUEST_HEADERS.
    assert!(head.contains("\r\naccept-encoding: gzip\r\n"), "{head}");
    assert!(
        head.contains("\r\nuser-agent: bazel/release 9.3.2\r\n"),
        "{head}"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/bazel/repository/downloader/HttpConnectorTest.java:166 HttpConnectorTest.serverError_retriesConnect
#[test]
fn server_error_retries_connect() {
    let error = lines(&[
        "HTTP/1.1 500 Incredible Catastrophe",
        "Connection: close",
        "Content-Length: 8",
        "",
        "nononono",
    ]);
    let mut server = Server::start(vec![error, ok_response("hello")]);
    let (f, handler) = fetcher(None, 0);
    let content = f
        .download_and_read_one_url_for_bzlmod(&server.url("/"), &no_env(), None)
        .unwrap();
    assert_eq!(content, b"hello");
    assert_eq!(server.requests().len(), 2);
    let events = handler.events.lock().unwrap().clone();
    assert!(
        events
            .iter()
            .any(|e| e.contains("Failed to connect to http://127.0.0.1:")),
        "{events:?}"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/bazel/repository/downloader/HttpConnectorTest.java:386 HttpConnectorTest.permanentError_doesNotRetryAndThrowsIOException
#[test]
fn permanent_error_does_not_retry_and_throws_io_exception() {
    let mut server = Server::start(vec![lines(&[
        "HTTP/1.1 401 Unauthorized",
        "Connection: close",
        "Content-Length: 0",
        "",
        "",
    ])]);
    let (f, _) = fetcher(None, 5);
    let e = f
        .download_and_read_one_url_for_bzlmod(&server.url("/"), &no_env(), None)
        .unwrap_err();
    assert_eq!(
        e,
        RegistryException::Io("GET returned 401 Unauthorized".to_string())
    );
    assert_eq!(server.requests().len(), 1);
}

// Java: src/test/java/com/google/devtools/build/lib/bazel/repository/downloader/HttpConnectorTest.java:594 HttpConnectorTest.redirectToDifferentPath_works
#[test]
fn redirect_to_different_path_works() {
    for code in [
        "301 Moved Permanently",
        "302 Found",
        "303 See Other",
        "307 Temporary Redirect",
    ] {
        let mut server = Server::start(vec![
            lines(&[
                &format!("HTTP/1.1 {code}"),
                "Location: /doodle.tar.gz",
                "Connection: close",
                "Content-Length: 0",
                "",
                "",
            ]),
            ok_response("hello"),
        ]);
        let (f, _) = fetcher(None, 0);
        let content = f
            .download_and_read_one_url_for_bzlmod(&server.url("/"), &no_env(), None)
            .unwrap();
        assert_eq!(content, b"hello", "{code}");
        let requests = server.requests();
        assert!(
            requests[1].starts_with("GET /doodle.tar.gz "),
            "{requests:?}"
        );
        // The final URL looks compressed, so no Accept-Encoding is sent.
        assert!(
            !requests[1].to_ascii_lowercase().contains("accept-encoding"),
            "{requests:?}"
        );
    }
}

// Java: src/test/java/com/google/devtools/build/lib/bazel/repository/downloader/HttpConnectorTest.java:107 HttpConnectorTest.localFileDownload
#[test]
fn local_file_download() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("MODULE.bazel");
    std::fs::write(&file, "module(name='x')\n").unwrap();
    let (f, _) = fetcher(None, 0);
    let url = JavaUri::new(&format!("file://{}", file.display())).unwrap();
    let content = f
        .download_and_read_one_url_for_bzlmod(&url, &no_env(), None)
        .unwrap();
    assert_eq!(content, b"module(name='x')\n");
}

#[test]
fn local_file_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("missing.json");
    let (f, _) = fetcher(None, 0);
    let url = JavaUri::new(&format!("file://{}", file.display())).unwrap();
    let e = f
        .download_and_read_one_url_for_bzlmod(&url, &no_env(), None)
        .unwrap_err();
    assert_eq!(
        e,
        RegistryException::NotFound(format!("{} (No such file or directory)", file.display()))
    );
}

// HttpStreamTest-style: a gzip Content-Encoding is decoded transparently.
#[test]
fn gzip_content_encoding_is_decoded() {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(b"hello gzip").unwrap();
    let gz = encoder.finish().unwrap();
    let mut response = lines(&[
        "HTTP/1.1 200 OK",
        "Connection: close",
        "Content-Encoding: gzip",
        &format!("Content-Length: {}", gz.len()),
        "",
        "",
    ]);
    response.extend_from_slice(&gz);
    let server = Server::start(vec![response]);
    let (f, _) = fetcher(None, 0);
    let content = f
        .download_and_read_one_url_for_bzlmod(
            &server.url("/source.json"),
            &no_env(),
            Some(&sha256(b"hello gzip")),
        )
        .unwrap();
    assert_eq!(content, b"hello gzip");
}

// ---------------------------------------------------------------------------------------------
// DownloadManager.downloadAndReadOneUrlForBzlmod and the repository cache
// ---------------------------------------------------------------------------------------------

#[test]
fn cache_hit_with_checksum_does_not_use_network() {
    let cache = tempfile::tempdir().unwrap();
    let hex = sha256_hex(b"cached");
    let file = cache_file(cache.path(), &hex);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, b"cached").unwrap();

    // A listener nobody accepts on: any connection attempt would show up in its backlog.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = JavaUri::new(&format!(
        "http://127.0.0.1:{}/modules/a/1.0/MODULE.bazel",
        listener.local_addr().unwrap().port()
    ))
    .unwrap();

    let (f, handler) = fetcher(Some(cache.path().to_path_buf()), 0);
    let content = f
        .download_and_read_one_url_for_bzlmod(&url, &no_env(), Some(&sha256(b"cached")))
        .unwrap();
    assert_eq!(content, b"cached");
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    assert_eq!(
        handler.cache_hits(),
        vec![DownloadCacheHitEvent {
            context: "Bazel module fetching".to_string(),
            file_hash: hex,
            uri: rf_uri(&url),
        }]
    );
}

#[test]
fn no_cache_lookup_without_checksum() {
    // Bazel only looks the file up when a checksum is given; without one it downloads.
    let cache = tempfile::tempdir().unwrap();
    let file = cache_file(cache.path(), &sha256_hex(b"fresh"));
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, b"fresh").unwrap();
    let mut server = Server::start(vec![ok_response("fresh")]);
    let (f, handler) = fetcher(Some(cache.path().to_path_buf()), 0);
    let content = f
        .download_and_read_one_url_for_bzlmod(&server.url("/x"), &no_env(), None)
        .unwrap();
    assert_eq!(content, b"fresh");
    assert_eq!(server.requests().len(), 1);
    assert!(handler.cache_hits().is_empty());
}

#[test]
fn download_without_checksum_puts_by_sha256() {
    let cache = tempfile::tempdir().unwrap();
    let server = Server::start(vec![ok_response("registry file")]);
    let (f, _) = fetcher(Some(cache.path().to_path_buf()), 0);
    f.download_and_read_one_url_for_bzlmod(&server.url("/bazel_registry.json"), &no_env(), None)
        .unwrap();
    let file = cache_file(cache.path(), &sha256_hex(b"registry file"));
    assert_eq!(std::fs::read(&file).unwrap(), b"registry file");
    // Only the value is left in the entry: the tmp-<uuid> file was renamed into place.
    let entries: Vec<_> = std::fs::read_dir(file.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(entries, vec![DEFAULT_CACHE_FILENAME.to_string()]);
}

#[test]
fn download_with_checksum_puts_and_then_hits() {
    let cache = tempfile::tempdir().unwrap();
    let mut server = Server::start(vec![ok_response("module file")]);
    let checksum = sha256(b"module file");
    let (f, handler) = fetcher(Some(cache.path().to_path_buf()), 0);
    let url = server.url("/modules/m/1/MODULE.bazel");
    assert_eq!(
        f.download_and_read_one_url_for_bzlmod(&url, &no_env(), Some(&checksum))
            .unwrap(),
        b"module file"
    );
    assert!(cache_file(cache.path(), &checksum.to_string()).exists());
    // The second read is served from the cache (the server only answers once).
    assert_eq!(
        f.download_and_read_one_url_for_bzlmod(&url, &no_env(), Some(&checksum))
            .unwrap(),
        b"module file"
    );
    assert_eq!(server.requests().len(), 1);
    assert_eq!(handler.cache_hits().len(), 1);
}

#[test]
fn poisoned_cache_entry_is_downloaded_again() {
    // DownloadCache.getBytes fails the checksum check; DownloadManager ignores the error,
    // downloads, and overwrites the entry.
    let cache = tempfile::tempdir().unwrap();
    let checksum = sha256(b"good");
    let file = cache_file(cache.path(), &checksum.to_string());
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, b"poisoned").unwrap();
    let mut server = Server::start(vec![ok_response("good")]);
    let (f, handler) = fetcher(Some(cache.path().to_path_buf()), 0);
    let content = f
        .download_and_read_one_url_for_bzlmod(&server.url("/x"), &no_env(), Some(&checksum))
        .unwrap();
    assert_eq!(content, b"good");
    assert_eq!(server.requests().len(), 1);
    assert!(handler.cache_hits().is_empty());
    assert_eq!(std::fs::read(&file).unwrap(), b"good");
}

#[test]
fn disabled_cache_writes_nothing() {
    let cache = tempfile::tempdir().unwrap();
    let server = Server::start(vec![ok_response("x")]);
    let (f, _) = fetcher(None, 0);
    f.download_and_read_one_url_for_bzlmod(&server.url("/x"), &no_env(), None)
        .unwrap();
    assert!(!cache.path().join(CAS_DIR).exists());
}

#[test]
fn checksum_mismatch_is_not_cached() {
    let cache = tempfile::tempdir().unwrap();
    let server = Server::start(vec![ok_response("evil")]);
    let checksum = sha256(b"good");
    let (f, _) = fetcher(Some(cache.path().to_path_buf()), 3);
    let e = f
        .download_and_read_one_url_for_bzlmod(&server.url("/x"), &no_env(), Some(&checksum))
        .unwrap_err();
    assert_eq!(
        e,
        RegistryException::Io(format!(
            "Checksum was {} but wanted {}",
            sha256_hex(b"evil"),
            checksum
        ))
    );
    assert!(!cache_file(cache.path(), &checksum.to_string()).exists());
}
