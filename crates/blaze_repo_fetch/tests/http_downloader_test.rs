//! Port of `HttpDownloaderTest`.
//!
//! The Java test mocks `Downloader`, `DownloadCache` and `UrlRewriter` with Mockito. Here the
//! mocked `Downloader` is a small fake implementing the trait, the mocked `DownloadCache` is a
//! real cache directory (disabled unless the test enables it), and the mocked `UrlRewriter` is a
//! real rewriter whose config produces the same URL list.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::thread;

use blaze_events::StoredEventHandler;
use blaze_events::handler::ExtendedEventHandler;
use blaze_repo_fetch::authandtls::{Credentials, Headers, StaticCredentials};
use blaze_repo_fetch::cache::{DownloadCache, KeyType};
use blaze_repo_fetch::downloader::{
    Checksum, DownloadManager, Downloader, HttpDownloader, UrlRewriter,
};
use blaze_repo_fetch::errors::{IoException, IoExceptionKind};
use blaze_repo_fetch::fs_utils;
use blaze_repo_fetch::uri::JavaUri;
use blaze_vfs::{DigestHashFunction, FileSystemRef, Path, UnixFileSystem};
use sha2::Digest as _;

// ---------------------------------------------------------------------------------------------
// Test utilities (ports of HttpParser.readHttpRequest and DownloaderTestUtils.sendLines).

/// Reads one HTTP request head (up to and including the blank line).
fn read_http_request(socket: &TcpStream) {
    let mut reader = BufReader::new(socket);
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).expect("read request");
        if n == 0 {
            panic!("IOException: unexpected EOF reading request");
        }
        if line == "\r\n" || line == "\n" {
            return;
        }
    }
}

/// Writes `data` joined with CRLF (no trailing CRLF), as ISO-8859-1.
fn send_lines(mut socket: &TcpStream, data: &[&str]) {
    let s = data.join("\r\n");
    let bytes: Vec<u8> = s.chars().map(|c| c as u32 as u8).collect();
    let _ = socket.write_all(&bytes);
    let _ = socket.flush();
}

fn bind() -> TcpListener {
    TcpListener::bind("127.0.0.1:0").unwrap()
}

/// Serves one request with `lines`, then closes the connection.
fn serve_once(server: TcpListener, lines: &'static [&'static str]) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        if let Ok((socket, _)) = server.accept() {
            read_http_request(&socket);
            send_lines(&socket, lines);
        }
    })
}

/// Serves every request with `lines` until the process exits.
fn serve_forever(server: TcpListener, lines: &'static [&'static str]) {
    thread::spawn(move || {
        for socket in server.incoming() {
            let Ok(socket) = socket else { continue };
            read_http_request(&socket);
            send_lines(&socket, lines);
        }
    });
}

fn url(server: &TcpListener, path: &str) -> JavaUri {
    JavaUri::create(&format!(
        "http://localhost:{}{}",
        server.local_addr().unwrap().port(),
        path
    ))
    .unwrap()
}

struct Fixture {
    _working_dir: tempfile::TempDir,
    fs: FileSystemRef,
    root: Path,
    counter: AtomicI32,
    event_handler: Arc<dyn ExtendedEventHandler>,
    http_downloader: Arc<HttpDownloader>,
}

impl Fixture {
    fn new() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        let root = Path::create_from_str(dir.path().to_str().unwrap(), &fs);
        Fixture {
            _working_dir: dir,
            fs,
            root,
            counter: AtomicI32::new(0),
            event_handler: Arc::new(StoredEventHandler::new()),
            // Scale timeouts down to make test fast.
            http_downloader: Arc::new(HttpDownloader::new(0, std::time::Duration::ZERO, 8, 0.1)),
        }
    }

    /// `workingDir.newFile()`.
    fn new_file(&self) -> Path {
        let p = self.root.get_child(&format!(
            "junit{}.tmp",
            self.counter.fetch_add(1, Ordering::SeqCst)
        ));
        fs_utils::write_content(&p, b"").unwrap();
        p
    }

    /// `workingDir.newFolder()`.
    fn new_folder(&self) -> Path {
        let p = self.root.get_child(&format!(
            "junit{}",
            self.counter.fetch_add(1, Ordering::SeqCst)
        ));
        p.create_directory_and_parents().unwrap();
        p
    }

    fn download_manager(
        &self,
        downloader: Arc<dyn Downloader>,
        cache: DownloadCache,
    ) -> Arc<DownloadManager> {
        Arc::new(DownloadManager::new(
            cache,
            downloader,
            self.http_downloader.clone(),
            self.event_handler.clone(),
        ))
    }

    fn default_manager(&self) -> Arc<DownloadManager> {
        self.download_manager(self.http_downloader.clone(), DownloadCache::new())
    }
}

/// The `download` helper: `startDownload` + `finalizeDownload`.
fn download(
    download_manager: &Arc<DownloadManager>,
    original_urls: Vec<JavaUri>,
    checksum: Option<Checksum>,
    canonical_id: &str,
    type_: Option<&str>,
    output: Path,
    context: &str,
) -> Result<Path, IoException> {
    let future = download_manager.start_download(
        original_urls,
        Headers::new(),
        HashMap::new(),
        checksum,
        canonical_id.to_string(),
        type_.map(str::to_string),
        output,
        HashMap::new(),
        context.to_string(),
        /* may_hardlink= */ true,
    );
    DownloadManager::finalize_download(future)
}

fn read_string(p: &Path) -> String {
    String::from_utf8(fs_utils::read_content(p).unwrap()).unwrap()
}

fn empty_creds() -> Arc<dyn Credentials> {
    Arc::new(StaticCredentials::empty())
}

/// A `Downloader` standing in for the Java test's Mockito mock: counts calls and runs `answer`.
struct FakeDownloader<F: Fn(i32, &Path) -> Result<(), IoException> + Send + Sync> {
    times: AtomicI32,
    answer: F,
}

impl<F: Fn(i32, &Path) -> Result<(), IoException> + Send + Sync> FakeDownloader<F> {
    fn new(answer: F) -> Arc<Self> {
        Arc::new(FakeDownloader {
            times: AtomicI32::new(0),
            answer,
        })
    }
}

impl<F: Fn(i32, &Path) -> Result<(), IoException> + Send + Sync> Downloader for FakeDownloader<F> {
    fn download(
        &self,
        _urls: &[JavaUri],
        _headers: &Headers,
        _credentials: Arc<dyn Credentials>,
        _checksum: Option<&Checksum>,
        _canonical_id: &str,
        output: &Path,
        _event_handler: Arc<dyn ExtendedEventHandler>,
        _client_env: &HashMap<String, String>,
        _type: Option<&str>,
        context: &str,
    ) -> Result<(), IoException> {
        // Mockito only answers for eq("testRepo"); any other context returns null (no-op).
        if context != "testRepo" {
            return Ok(());
        }
        let n = self.times.fetch_add(1, Ordering::SeqCst);
        (self.answer)(n, output)
    }
}

fn write_content_answer(output: &Path) -> Result<(), IoException> {
    fs_utils::write_content(output, b"content")?;
    Ok(())
}

const HELLO_200: &[&str] = &[
    "HTTP/1.1 200 OK",
    "Date: Fri, 31 Dec 1999 23:59:59 GMT",
    "Connection: close",
    "Content-Type: text/plain",
    "Content-Length: 5",
    "",
    "hello",
];

const CONTENT1_200: &[&str] = &[
    "HTTP/1.1 200 OK",
    "Date: Fri, 31 Dec 1999 23:59:59 GMT",
    "Connection: close",
    "Content-Type: text/plain",
    "",
    "content1",
];

const CONTENT2_200: &[&str] = &[
    "HTTP/1.1 200 OK",
    "Date: Fri, 31 Dec 1999 23:59:59 GMT",
    "Connection: close",
    "Content-Type: text/plain",
    "",
    "content2",
];

const NOT_FOUND_5: &[&str] = &[
    "HTTP/1.1 404 Not Found",
    "Date: Fri, 31 Dec 1999 23:59:59 GMT",
    "Connection: close",
    "Content-Type: text/plain",
    "Content-Length: 5",
    "",
    "",
];

const NOT_FOUND_0: &[&str] = &[
    "HTTP/1.1 404 Not Found",
    "Date: Fri, 31 Dec 1999 23:59:59 GMT",
    "Connection: close",
    "Content-Type: text/plain",
    "Content-Length: 0",
    "",
    "",
];

// ---------------------------------------------------------------------------------------------

#[test]
fn download_from1_url_ok() {
    let f = Fixture::new();
    let server = bind();
    let u = url(&server, "/foo");
    let _h = serve_once(server, HELLO_200);
    let resulting_file = download(
        &f.default_manager(),
        vec![u],
        None,
        "testCanonicalId",
        None,
        f.new_file(),
        "testRepo",
    )
    .unwrap();
    assert_eq!(read_string(&resulting_file), "hello");
}

#[test]
fn download_from1_url_ok_special_char_in_basename() {
    let f = Fixture::new();
    let server = bind();
    let u = url(&server, "/arch:ve.zip");
    let _h = serve_once(server, HELLO_200);
    let resulting_file = download(
        &f.default_manager(),
        vec![u],
        None,
        "testCanonicalId",
        Some("zip"),
        f.new_folder(),
        "testRepo",
    )
    .unwrap();
    assert_eq!(read_string(&resulting_file), "hello");
    assert_eq!(resulting_file.as_fragment().get_file_extension(), "zip");
    assert!(!resulting_file.as_fragment().get_base_name().contains(':'));
}

#[test]
fn download_from_opaque_file_url_with_explicit_output() {
    let f = Fixture::new();
    let downloader = FakeDownloader::new(|_, output| write_content_answer(output));
    let dm = f.download_manager(downloader, DownloadCache::new());
    let result = download(
        &dm,
        vec![JavaUri::create("file:../vendored_lodash-4.17.21.tgz").unwrap()],
        None,
        "testCanonicalId",
        None,
        f.new_file(),
        "testRepo",
    )
    .unwrap();
    assert_eq!(read_string(&result), "content");
}

#[test]
fn download_from_opaque_file_url_with_type() {
    let f = Fixture::new();
    let downloader = FakeDownloader::new(|_, output| write_content_answer(output));
    let dm = f.download_manager(downloader, DownloadCache::new());
    let result = download(
        &dm,
        vec![JavaUri::create("file:../vendored_lodash-4.17.21.tgz").unwrap()],
        None,
        "testCanonicalId",
        Some("tgz"),
        f.new_folder(),
        "testRepo",
    )
    .unwrap();
    assert_eq!(result.get_base_name(), "vendored_lodash-4.17.21.tgz");
    assert_eq!(read_string(&result), "content");
}

#[test]
fn download_from_opaque_file_url_with_escaped_question_mark_and_type() {
    let f = Fixture::new();
    let downloader = FakeDownloader::new(|_, output| write_content_answer(output));
    let dm = f.download_manager(downloader, DownloadCache::new());
    let result = download(
        &dm,
        vec![JavaUri::create("file:../foo%3Fbar.tgz").unwrap()],
        None,
        "testCanonicalId",
        Some("tgz"),
        f.new_folder(),
        "testRepo",
    )
    .unwrap();
    assert_eq!(result.get_base_name(), "foo_bar.tgz");
    assert_eq!(read_string(&result), "content");
}

#[test]
fn get_candidate_file_names_opaque_file_url_with_escaped_question_mark() {
    let f = Fixture::new();
    let names = blaze_repo_fetch::downloader::download_manager::get_candidate_file_names(
        &JavaUri::create("file:../foo%3Fbar.tgz").unwrap(),
        &Path::create_from_str("/tmp/foo_bar.tgz", &f.fs),
    );
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    assert_eq!(names, vec!["foo?bar.tgz", "foo_bar.tgz"]);
}

#[test]
fn rewriter_blocks_all_urls_cache_hit_preserves_url_file_name() {
    // When an all-blocking --downloader_config removes every URL, the request can only be served
    // from the repository cache by checksum. The resulting file name must still be derived from
    // the original URL so that its extension is preserved.
    let f = Fixture::new();
    let block_all = UrlRewriter::new(&["/dev/null".to_string()], &["block *".to_string()]).unwrap();
    let data = b"content";
    let hash = hex::encode(sha2::Sha256::digest(data));
    // PORT: the Java test mocks DownloadCache.get to write the data; here the data is put into a
    // real (enabled) cache.
    let cache_dir = f.new_folder();
    let mut cache = DownloadCache::new();
    cache.set_path(Some(cache_dir));
    cache.put_bytes(data, KeyType::SHA256).unwrap();
    let mut dm = DownloadManager::new(
        cache,
        f.http_downloader.clone(),
        f.http_downloader.clone(),
        f.event_handler.clone(),
    );
    dm.set_url_rewriter(Some(block_all));
    let dm = Arc::new(dm);
    let checksum = Checksum::from_string(KeyType::SHA256, &hash).unwrap();
    let result = download(
        &dm,
        vec![JavaUri::create("http://mirror.example/archive.tar.gz").unwrap()],
        Some(checksum),
        // PORT: the mock matched any canonical id; the real cache only matches entries stored
        // with the same id, so the lookup uses none.
        "",
        // download_and_extract passes an (often empty) type and relies on the file extension.
        Some(""),
        f.new_folder(),
        "testRepo",
    )
    .unwrap();
    assert_eq!(result.get_base_name(), "archive.tar.gz");
}

#[test]
fn download_from2_urls_first_ok() {
    let f = Fixture::new();
    let server1 = bind();
    let server2 = bind();
    let urls = vec![url(&server1, "/foo"), url(&server2, "/foo")];
    serve_forever(server1, CONTENT1_200);
    serve_forever(server2, CONTENT2_200);
    let resulting_file = download(
        &f.default_manager(),
        urls,
        None,
        "testCanonicalId",
        None,
        f.new_file(),
        "testRepo",
    )
    .unwrap();
    assert_eq!(read_string(&resulting_file), "content1");
}

#[test]
#[ignore = "b/182150157 (ignored in the Java test too)"]
fn download_from2_urls_first_socket_timeout_on_body_read_second_ok() {
    let f = Fixture::new();
    let server1 = bind();
    let server2 = bind();
    let urls = vec![url(&server1, "/foo"), url(&server2, "/foo")];
    thread::spawn(move || {
        let (socket, _) = server1.accept().unwrap();
        read_http_request(&socket);
        send_lines(&socket, CONTENT1_200);
        // Never close the socket to cause SocketTimeoutException during body read on client side.
        std::mem::forget(socket);
    });
    serve_forever(server2, CONTENT2_200);
    let resulting_file = download(
        &f.default_manager(),
        urls,
        None,
        "testCanonicalId",
        None,
        f.new_file(),
        "testRepo",
    )
    .unwrap();
    assert_eq!(read_string(&resulting_file), "content2");
}

#[test]
#[ignore = "b/182150157 (ignored in the Java test too)"]
fn download_from2_urls_both_socket_timeout_during_body_read() {
    let f = Fixture::new();
    let server1 = bind();
    let server2 = bind();
    let urls = vec![url(&server1, "/foo"), url(&server2, "/foo")];
    thread::spawn(move || {
        for _ in 0..2 {
            let (socket, _) = server1.accept().unwrap();
            read_http_request(&socket);
            send_lines(&socket, CONTENT1_200);
            std::mem::forget(socket);
        }
    });
    let e = download(
        &f.default_manager(),
        urls,
        None,
        "testCanonicalId",
        None,
        f.new_file(),
        "testRepo",
    )
    .unwrap_err();
    drop(server2);
    assert_eq!(e.suppressed.len(), 2);
    for s in &e.suppressed {
        assert_eq!(s.kind, IoExceptionKind::IOException);
        assert!(s.get_message().contains("SocketTimeoutException"));
    }
}

#[test]
fn download_from2_urls_first_tls_error_second_ok() {
    let f = Fixture::new();
    let server1 = bind();
    let server2 = bind();
    let urls = vec![
        // Use https for the first one to trigger SSL handshake
        JavaUri::create(&format!(
            "https://localhost:{}/foo",
            server1.local_addr().unwrap().port()
        ))
        .unwrap(),
        url(&server2, "/foo"),
    ];
    let server1_future = thread::spawn(move || {
        let (mut socket, _) = server1.accept().unwrap();
        // Write garbage to trigger SSL handshake failure on client
        let _ = socket.write_all(b"Not SSL");
    });
    let server2_future = serve_once(server2, CONTENT2_200);
    let resulting_file = f.new_file();
    f.http_downloader
        .download(
            &urls,
            &Headers::new(),
            empty_creds(),
            None,
            "testCanonicalId",
            &resulting_file,
            f.event_handler.clone(),
            &HashMap::new(),
            None,
            "testRepo",
        )
        .unwrap();
    server1_future.join().unwrap();
    server2_future.join().unwrap();
    assert_eq!(read_string(&resulting_file), "content2");
}

#[test]
fn download_one_url_ok() {
    let f = Fixture::new();
    let server = bind();
    let u = url(&server, "/foo");
    let _h = serve_once(server, HELLO_200);
    let destination = f.new_file();
    f.http_downloader
        .download(
            &[u],
            &Headers::new(),
            empty_creds(),
            None,
            "testCanonicalId",
            &destination,
            f.event_handler.clone(),
            &HashMap::new(),
            None,
            "context",
        )
        .unwrap();
    assert_eq!(read_string(&destination), "hello");
}

#[test]
fn download_one_url_not_found() {
    let f = Fixture::new();
    let server = bind();
    let u = url(&server, "/foo");
    let _h = serve_once(server, NOT_FOUND_5);
    let r = f.http_downloader.download(
        &[u],
        &Headers::new(),
        empty_creds(),
        None,
        "testCanonicalId",
        &f.new_file(),
        f.event_handler.clone(),
        &HashMap::new(),
        None,
        "context",
    );
    assert!(r.is_err());
}

#[test]
fn download_two_urls_first_not_found_and_second_ok() {
    let f = Fixture::new();
    let server1 = bind();
    let server2 = bind();
    let urls = vec![url(&server1, "/foo"), url(&server2, "/foo")];
    let _h = serve_once(server1, NOT_FOUND_5);
    serve_forever(server2, CONTENT2_200);
    let destination = f.new_file();
    f.http_downloader
        .download(
            &urls,
            &Headers::new(),
            empty_creds(),
            None,
            "testCanonicalId",
            &destination,
            f.event_handler.clone(),
            &HashMap::new(),
            None,
            "context",
        )
        .unwrap();
    assert_eq!(read_string(&destination), "content2");
}

#[test]
fn download_and_read_ok() {
    let f = Fixture::new();
    let server = bind();
    let u = url(&server, "/foo");
    let _h = serve_once(server, HELLO_200);
    let content = f
        .http_downloader
        .download_and_read(
            &[u],
            empty_creds(),
            None,
            f.event_handler.clone(),
            &HashMap::new(),
        )
        .unwrap();
    assert_eq!(String::from_utf8(content).unwrap(), "hello");
}

#[test]
fn download_and_read_not_found() {
    let f = Fixture::new();
    let server = bind();
    let u = url(&server, "/foo");
    let _h = serve_once(server, NOT_FOUND_5);
    let r = f.http_downloader.download_and_read(
        &[u],
        empty_creds(),
        None,
        f.event_handler.clone(),
        &HashMap::new(),
    );
    assert!(r.is_err());
}

#[test]
fn download_and_read_checksum_provided() {
    let f = Fixture::new();
    let server = bind();
    let u = url(&server, "/foo");
    let _h = serve_once(server, HELLO_200);
    let checksum = Checksum::from_string(
        KeyType::SHA256,
        &hex::encode(sha2::Sha256::digest(b"hello")),
    )
    .unwrap();
    let content = f
        .http_downloader
        .download_and_read(
            &[u],
            empty_creds(),
            Some(&checksum),
            f.event_handler.clone(),
            &HashMap::new(),
        )
        .unwrap();
    assert_eq!(String::from_utf8(content).unwrap(), "hello");
}

#[test]
fn download_and_read_checksum_mismatch() {
    let f = Fixture::new();
    let server = bind();
    let u = url(&server, "/foo");
    let _h = serve_once(
        server,
        &[
            "HTTP/1.1 200 OK",
            "Date: Fri, 31 Dec 1999 23:59:59 GMT",
            "Connection: close",
            "Content-Type: text/plain",
            "Content-Length: 9",
            "",
            "malicious",
        ],
    );
    // Hashing.sha256().hashUnencodedChars("hello"): the UTF-16LE code units.
    let utf16: Vec<u8> = "hello"
        .encode_utf16()
        .flat_map(|u| u.to_le_bytes())
        .collect();
    let checksum =
        Checksum::from_string(KeyType::SHA256, &hex::encode(sha2::Sha256::digest(&utf16))).unwrap();
    let e = f
        .http_downloader
        .download_and_read(
            &[u],
            empty_creds(),
            Some(&checksum),
            f.event_handler.clone(),
            &HashMap::new(),
        )
        .unwrap_err();
    assert_eq!(e.kind, IoExceptionKind::UnrecoverableHttpException);
    assert!(e.get_message().contains("Checksum was"), "{}", e);
}

#[test]
fn download_and_read_two_urls_first_not_found_and_second_ok() {
    let f = Fixture::new();
    let server1 = bind();
    let server2 = bind();
    let urls = vec![url(&server1, "/foo"), url(&server2, "/foo")];
    let _h = serve_once(server1, NOT_FOUND_0);
    serve_forever(server2, CONTENT2_200);
    let content = f
        .http_downloader
        .download_and_read(
            &urls,
            empty_creds(),
            None,
            f.event_handler.clone(),
            &HashMap::new(),
        )
        .unwrap();
    assert_eq!(String::from_utf8(content).unwrap(), "content2");
}

#[test]
fn download_and_read_one_url_for_bzlmod_falls_back_to_next_rewritten_url() {
    let f = Fixture::new();
    let primary = bind();
    let fallback = bind();
    let primary_port = primary.local_addr().unwrap().port();
    let fallback_port = fallback.local_addr().unwrap().port();
    // The first (e.g. proxy/mirror) URL fails ...
    let _h = serve_once(primary, NOT_FOUND_0);
    // ... so the read must fall back to the second (e.g. direct) URL.
    serve_forever(fallback, CONTENT2_200);

    let original_url = JavaUri::create("https://example.com/foo").unwrap();
    // PORT: the Java test mocks UrlRewriter.amend to return [primary, fallback] (rewritten) and
    // updateAuthHeaders to return no headers. Two rewrite rules for the same pattern produce
    // exactly that list; without netrc credentials or user info no auth headers are added.
    let rewriter = UrlRewriter::new(
        &["config".to_string()],
        &[format!(
            "rewrite example.com/(.*) http://localhost:{}/$1\nrewrite example.com/(.*) http://localhost:{}/$1",
            primary_port, fallback_port
        )],
    )
    .unwrap();
    let mut dm = DownloadManager::new(
        DownloadCache::new(),
        f.http_downloader.clone(),
        f.http_downloader.clone(),
        f.event_handler.clone(),
    );
    dm.set_url_rewriter(Some(rewriter));
    let content = dm
        .download_and_read_one_url_for_bzlmod(&original_url, &HashMap::new(), None)
        .unwrap();
    assert_eq!(String::from_utf8(content).unwrap(), "content2");
}

fn content_length_mismatch() -> IoException {
    IoException::of(
        IoExceptionKind::ContentLengthMismatchException,
        format!("Bytes read {} but wanted {}", 0, 7),
    )
}

#[test]
fn download_content_length_mismatch_propagate_error_if_not_retry() {
    let f = Fixture::new();
    let downloader = FakeDownloader::new(|_, _| Err(content_length_mismatch()));
    let mut dm = DownloadManager::new(
        DownloadCache::new(),
        downloader.clone(),
        f.http_downloader.clone(),
        f.event_handler.clone(),
    );
    // do not retry
    dm.set_retries(0);
    let e = download(
        &Arc::new(dm),
        vec![JavaUri::create("http://localhost").unwrap()],
        None,
        "testCanonicalId",
        None,
        f.new_file(),
        "testRepo",
    )
    .unwrap_err();
    assert_eq!(e.kind, IoExceptionKind::ContentLengthMismatchException);
    assert_eq!(downloader.times.load(Ordering::SeqCst), 1);
}

fn run_retry_test<F: Fn(i32, &Path) -> Result<(), IoException> + Send + Sync + 'static>(answer: F) {
    let f = Fixture::new();
    let downloader = FakeDownloader::new(answer);
    let mut dm = DownloadManager::new(
        DownloadCache::new(),
        downloader.clone(),
        f.http_downloader.clone(),
        f.event_handler.clone(),
    );
    dm.set_retries(5);
    let result = download(
        &Arc::new(dm),
        vec![JavaUri::create("http://localhost").unwrap()],
        None,
        "testCanonicalId",
        None,
        f.new_file(),
        "testRepo",
    )
    .unwrap();
    assert_eq!(downloader.times.load(Ordering::SeqCst), 4);
    assert_eq!(read_string(&result), "content");
}

#[test]
fn download_content_length_mismatch_retries() {
    run_retry_test(|n, output| {
        if n < 3 {
            return Err(content_length_mismatch());
        }
        write_content_answer(output)
    });
}

#[test]
fn download_content_length_mismatch_with_other_errors_retries() {
    run_retry_test(|n, output| {
        if n < 3 {
            let mut e = IoException::without_message(IoExceptionKind::IOException);
            e.add_suppressed(content_length_mismatch());
            e.add_suppressed(IoException::without_message(IoExceptionKind::IOException));
            return Err(e);
        }
        write_content_answer(output)
    });
}

#[test]
fn download_socket_exception_retries() {
    run_retry_test(|n, output| {
        if n < 3 {
            return Err(IoException::of(
                IoExceptionKind::SocketException,
                "Connection reset",
            ));
        }
        write_content_answer(output)
    });
}

#[test]
fn download_socket_exception_with_other_errors_retries() {
    run_retry_test(|n, output| {
        if n < 3 {
            let mut e = IoException::without_message(IoExceptionKind::IOException);
            e.add_suppressed(IoException::of(
                IoExceptionKind::SocketException,
                "Connection reset",
            ));
            e.add_suppressed(IoException::without_message(IoExceptionKind::IOException));
            return Err(e);
        }
        write_content_answer(output)
    });
}
