//! A chunked HTTP response that the server cuts off (review 2026-11-18 round 2, N1), through
//! the real socket transport and a local server.
//!
//! Java: `sun.net.www.http.ChunkedInputStream` throws `IOException("Premature EOF")` when the
//! connection ends before the terminating chunk. What happens next depends on the server
//! (HttpStream.java:73-81): only a response with `Accept-Ranges: bytes` is wrapped in a
//! `RetryingInputStream`, which reconnects (RetryingInputStream.java:97-127) from the start if
//! nothing had been read, else with a `Range` header, failing with "Tried to reconnect at offset
//! N but server didn't support it" when the answer has no matching `Content-Range`. Without
//! `Accept-Ranges` the "Premature EOF" is the download's error. `DownloadManager
//! .isRetryableException` retries neither. In every case the truncated bytes are never returned
//! or cached, also when no checksum is given.

mod downloader_support;

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use blaze_repo_fetch::cache::RepositoryCache;
use blaze_repo_fetch::downloader::{DelegatingDownloader, DownloadManager, HttpDownloader};
use blaze_repo_fetch::errors::IoExceptionKind;
use blaze_repo_fetch::uri::JavaUri;
use blaze_vfs::{DigestHashFunction, FileSystemRef, Path, UnixFileSystem};
use downloader_support::RecordingHandler;
use indexmap::IndexMap;

fn fs() -> FileSystemRef {
    UnixFileSystem::create(DigestHashFunction::Sha256)
}

/// A server on 127.0.0.1 that answers every connection with the next of `responses` (the last
/// one again once they run out) and then closes the connection. Returns the port and the
/// request heads it has seen.
fn serve(responses: Vec<&'static str>) -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = requests.clone();
    std::thread::spawn(move || {
        for (served, socket) in listener.incoming().enumerate() {
            let Ok(mut socket) = socket else { return };
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                match socket.read(&mut byte) {
                    Ok(1) => head.push(byte[0]),
                    _ => break,
                }
            }
            recorded
                .lock()
                .unwrap()
                .push(String::from_utf8_lossy(&head).to_ascii_lowercase());
            let response = responses[served.min(responses.len() - 1)];
            let _ = socket.write_all(response.as_bytes());
            let _ = socket.flush();
            let _ = socket.shutdown(std::net::Shutdown::Both);
        }
    });
    (port, requests)
}

/// The header of the second chunk announces 0x14 bytes; 5 arrive before the connection closes.
const CUT_INSIDE_A_CHUNK: &str =
    "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n6\r\nfirst \r\n14\r\nhello";
/// The connection closes after a complete chunk, before the terminating chunk.
const CUT_BETWEEN_CHUNKS: &str =
    "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n6\r\nfirst \r\n";
/// The connection closes after the last chunk, before the empty line that ends the trailer.
const CUT_BEFORE_THE_END_OF_THE_TRAILER: &str = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n6\r\nfirst \r\n14\r\nhello, chunked world\r\n0\r\n";
/// As [`CUT_INSIDE_A_CHUNK`], from a server that announces range support.
const RANGES_CUT_INSIDE_A_CHUNK: &str = "HTTP/1.1 200 OK\r\nAccept-Ranges: bytes\r\nTransfer-Encoding: chunked\r\n\r\n6\r\nfirst \r\n14\r\nhello";
/// The connection closes inside the first chunk-size line: no body byte was delivered.
const RANGES_CUT_BEFORE_THE_FIRST_CHUNK: &str =
    "HTTP/1.1 200 OK\r\nAccept-Ranges: bytes\r\nTransfer-Encoding: chunked\r\n\r\n6";
const COMPLETE: &str = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n6\r\nfirst \r\n14\r\nhello, chunked world\r\n0\r\n\r\n";
const CONTENT: &[u8] = b"first hello, chunked world";

struct Fixture {
    _tmp: tempfile::TempDir,
    cache_dir: std::path::PathBuf,
    out_dir: Path,
    manager: DownloadManager,
}

/// A download manager with the repository cache enabled, `HttpDownloaderTest`'s downloader
/// (timeouts scaled down) and the given `--experimental_repository_downloader_retries`.
fn fixture(retries: i32) -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let cache_dir = root.join("cache");
    let fs = fs();
    let mut repository_cache = RepositoryCache::new();
    repository_cache.set_path(Some(Path::create_from_str(
        cache_dir.to_str().unwrap(),
        &fs,
    )));
    let http_downloader = Arc::new(HttpDownloader::new(0, Duration::ZERO, 8, 0.1));
    let mut manager = DownloadManager::new(
        repository_cache.get_download_cache().clone(),
        Arc::new(DelegatingDownloader::new(http_downloader.clone())),
        http_downloader,
        RecordingHandler::new(),
    );
    manager.set_retries(retries);
    let out_dir = Path::create_from_str(root.join("out").to_str().unwrap(), &fs);
    out_dir.create_directory_and_parents().unwrap();
    Fixture {
        _tmp: tmp,
        cache_dir,
        out_dir,
        manager,
    }
}

impl Fixture {
    /// The keys in the repository cache's content-addressable store.
    fn cached(&self) -> Vec<String> {
        let dir = self.cache_dir.join("content_addressable/sha256");
        match std::fs::read_dir(dir) {
            Ok(entries) => entries
                .map(|e| e.unwrap().file_name().into_string().unwrap())
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    /// `DownloadManager.downloadAndReadOneUrlForBzlmod` without a checksum (a registry file
    /// that is not in the lockfile yet).
    fn read(&self, port: u16) -> Result<Vec<u8>, blaze_repo_fetch::errors::IoException> {
        self.manager.download_and_read_one_url_for_bzlmod(
            &JavaUri::create(&format!(
                "http://127.0.0.1:{port}/modules/a/1.0/source.json"
            ))
            .unwrap(),
            &HashMap::new(),
            None,
        )
    }

    /// `DownloadManager.download` without a checksum (`rctx.download` with no `sha256` or
    /// `integrity`).
    fn download(&self, port: u16) -> Result<Path, blaze_repo_fetch::errors::IoException> {
        self.manager.download(
            &[JavaUri::create(&format!("http://127.0.0.1:{port}/archive.txt")).unwrap()],
            &IndexMap::new(),
            &HashMap::new(),
            None,
            "",
            None,
            &self.out_dir.get_relative_str("archive.txt"),
            &HashMap::new(),
            "test",
            false,
        )
    }
}

#[test]
fn complete_chunked_response_is_read_and_cached() {
    let (port, _) = serve(vec![COMPLETE]);
    let f = fixture(0);
    assert_eq!(f.read(port).unwrap(), CONTENT);
    assert_eq!(f.cached().len(), 1);
}

/// No `Accept-Ranges`: the JDK's "Premature EOF" is the error, wherever the body was cut.
#[test]
fn bzlmod_read_of_truncated_chunked_response_fails_and_is_not_cached() {
    for response in [
        CUT_INSIDE_A_CHUNK,
        CUT_BETWEEN_CHUNKS,
        CUT_BEFORE_THE_END_OF_THE_TRAILER,
    ] {
        let (port, requests) = serve(vec![response]);
        let f = fixture(0);
        let e = f.read(port).unwrap_err();
        assert_eq!(e.kind, IoExceptionKind::IOException, "{response:?}");
        assert_eq!(e.get_message(), "Premature EOF", "{response:?}");
        assert_eq!(requests.lock().unwrap().len(), 1, "{response:?}");
        assert_eq!(f.cached(), Vec::<String>::new(), "{response:?}");
    }
}

/// `--experimental_repository_downloader_retries` does not change the outcome: the error is not
/// one `DownloadManager.isRetryableException` retries.
#[test]
fn bzlmod_read_of_truncated_response_is_not_retried_by_the_download_manager() {
    let (port, requests) = serve(vec![CUT_INSIDE_A_CHUNK, COMPLETE]);
    let f = fixture(3);
    let e = f.read(port).unwrap_err();
    assert_eq!(e.get_message(), "Premature EOF");
    assert_eq!(requests.lock().unwrap().len(), 1);
    assert_eq!(f.cached(), Vec::<String>::new());
}

/// `rctx.download` without a checksum: the error, no file in the cache.
#[test]
fn file_download_of_truncated_chunked_response_fails_and_is_not_cached() {
    let (port, _) = serve(vec![CUT_INSIDE_A_CHUNK]);
    let f = fixture(0);
    let e = f.download(port).unwrap_err();
    assert_eq!(
        e.get_message(),
        format!(
            "Error downloading [http://127.0.0.1:{port}/archive.txt] to {}/archive.txt: \
             Premature EOF",
            f.out_dir.get_path_string()
        )
    );
    assert_eq!(f.cached(), Vec::<String>::new());
}

/// `Accept-Ranges: bytes`: `RetryingInputStream` reconnects with `Range`, and a server that
/// does not answer with `Content-Range` ends the download.
#[test]
fn truncated_response_of_range_server_without_content_range_fails() {
    let (port, requests) = serve(vec![RANGES_CUT_INSIDE_A_CHUNK]);
    let f = fixture(0);
    let e = f.read(port).unwrap_err();
    // "first " and "hello" had been read.
    assert_eq!(
        e.get_message(),
        "Tried to reconnect at offset 11 but server didn't support it"
    );
    let requests = requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 2, "{requests:?}");
    assert!(
        requests[1].contains("\r\nrange: bytes=11-\r\n"),
        "{requests:?}"
    );
    assert_eq!(f.cached(), Vec::<String>::new());
}

/// Nothing had been read, so `RetryingInputStream` reconnects without `Range`, and the complete
/// second answer is used.
#[test]
fn response_cut_before_the_first_chunk_is_retried_from_the_start() {
    let (port, requests) = serve(vec![RANGES_CUT_BEFORE_THE_FIRST_CHUNK, COMPLETE]);
    let f = fixture(0);
    assert_eq!(f.read(port).unwrap(), CONTENT);
    let requests = requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 2, "{requests:?}");
    assert!(!requests[1].contains("\r\nrange:"), "{requests:?}");
    assert_eq!(f.cached().len(), 1);
}

/// A server that honours `Range` lets the download resume where the cut happened.
#[test]
fn response_cut_inside_a_chunk_resumes_with_range() {
    let (port, requests) = serve(vec![
        RANGES_CUT_INSIDE_A_CHUNK,
        "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 11-25/26\r\nContent-Length: 15\r\n\r\n, chunked world",
    ]);
    let f = fixture(0);
    assert_eq!(f.read(port).unwrap(), CONTENT);
    let requests = requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 2, "{requests:?}");
    assert!(
        requests[1].contains("\r\nrange: bytes=11-\r\n"),
        "{requests:?}"
    );
}
