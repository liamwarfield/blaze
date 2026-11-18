//! Request headers from `rctx.download(headers = ...)` and friends, as `HttpConnector` passes
//! them to `HttpURLConnection.addRequestProperty` (review 2026-11-18 round 3, R3-1).
//!
//! The JDK (25.0.2, the embedded one; `sun.net.www.protocol.http.HttpURLConnection`, read from
//! the class file):
//! - `checkMessageHeader`: a name with LF or `:`, or a value with LF not followed by space or
//!   tab, throws `IllegalArgumentException("Illegal character(s) in message header field: " +
//!   key)` / `("... value: " + value)`, which `HttpConnector.java:245-246` rethrows as
//!   `UnrecoverableHttpException` with the same message;
//! - `isRestrictedHeader`: `Host`, `Connection` (except `close`), `Content-Length`,
//!   `Transfer-Encoding`, `Upgrade`, `Via`, `Keep-Alive`, `Origin`, `Trailer`,
//!   `Content-Transfer-Encoding`, the two `Access-Control-Request-*` names and every `Sec-*`
//!   name are dropped silently. Bazel does not set `sun.net.http.allowRestrictedHeaders`.

mod downloader_support;

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use blaze_repo_fetch::cache::RepositoryCache;
use blaze_repo_fetch::downloader::http_connector::{
    IllegalArgumentException, RequestProperties, check_message_header, is_restricted_header,
};
use blaze_repo_fetch::downloader::{DelegatingDownloader, DownloadManager, HttpDownloader};
use blaze_repo_fetch::errors::IoException;
use blaze_repo_fetch::uri::JavaUri;
use blaze_vfs::{DigestHashFunction, Path, UnixFileSystem};
use downloader_support::RecordingHandler;
use indexmap::IndexMap;

/// A server on 127.0.0.1 that answers every connection with "ok" and records the raw request
/// heads.
fn serve() -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = requests.clone();
    std::thread::spawn(move || {
        for socket in listener.incoming() {
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
                .push(String::from_utf8_lossy(&head).into_owned());
            let _ = socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok");
            let _ = socket.shutdown(std::net::Shutdown::Both);
        }
    });
    (port, requests)
}

/// `rctx.download(url, headers = headers)` without a checksum, cache off, no retries.
fn download(port: u16, headers: &[(&str, &str)]) -> Result<Vec<u8>, IoException> {
    let tmp = tempfile::tempdir().unwrap();
    let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
    let http_downloader = Arc::new(HttpDownloader::new(0, Duration::ZERO, 8, 0.1));
    let manager = DownloadManager::new(
        RepositoryCache::new().get_download_cache().clone(),
        Arc::new(DelegatingDownloader::new(http_downloader.clone())),
        http_downloader,
        RecordingHandler::new(),
    );
    let mut request_headers: IndexMap<String, Vec<String>> = IndexMap::new();
    for (name, value) in headers {
        request_headers
            .entry(name.to_string())
            .or_default()
            .push(value.to_string());
    }
    let output = Path::create_from_str(
        tmp.path()
            .canonicalize()
            .unwrap()
            .join("out")
            .to_str()
            .unwrap(),
        &fs,
    );
    let path = manager.download(
        &[JavaUri::create(&format!("http://127.0.0.1:{port}/file.txt")).unwrap()],
        &request_headers,
        &HashMap::new(),
        None,
        "",
        None,
        &output,
        &HashMap::new(),
        "test",
        false,
    )?;
    Ok(std::fs::read(path.get_path_string()).unwrap())
}

/// The header lines of a request head, lower-cased names, without the request line.
fn header_lines(head: &str) -> Vec<(String, String)> {
    head.split("\r\n")
        .skip(1)
        .filter(|l| !l.is_empty())
        .map(|l| {
            let (name, value) = l.split_once(':').unwrap();
            (name.to_ascii_lowercase(), value.trim().to_string())
        })
        .collect()
}

/// Scenario 1 of R3-1: `headers = {"Host": "other.example"}` cannot override the URL's host.
#[test]
fn host_header_cannot_be_overridden() {
    let (port, requests) = serve();
    assert_eq!(
        download(
            port,
            &[("Host", "other.example"), ("hOSt", "third.example")]
        )
        .unwrap(),
        b"ok"
    );
    let requests = requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 1);
    let hosts: Vec<String> = header_lines(&requests[0])
        .into_iter()
        .filter(|(name, _)| name == "host")
        .map(|(_, value)| value)
        .collect();
    assert_eq!(hosts, vec![format!("127.0.0.1:{port}")]);
    assert!(!requests[0].contains("example"), "{requests:?}");
}

#[test]
fn restricted_headers_are_dropped_silently() {
    let (port, requests) = serve();
    assert_eq!(
        download(
            port,
            &[
                ("Transfer-Encoding", "chunked"),
                ("Content-Length", "5"),
                ("Connection", "upgrade"),
                ("Upgrade", "h2c"),
                ("Via", "1.1 evil"),
                ("Keep-Alive", "timeout=5"),
                ("Origin", "http://o.example"),
                ("Trailer", "X"),
                ("Content-Transfer-Encoding", "binary"),
                ("Access-Control-Request-Method", "PUT"),
                ("Access-Control-Request-Headers", "X"),
                ("Sec-Fetch-Mode", "cors"),
                ("X-Allowed", "yes"),
                ("Authorization", "Bearer tok"),
            ],
        )
        .unwrap(),
        b"ok"
    );
    let requests = requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 1);
    let mut names: Vec<String> = header_lines(&requests[0])
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "accept",
            "accept-encoding",
            "authorization",
            "connection",
            "host",
            "user-agent",
            "x-allowed",
        ]
    );
    // The transport's own keep-alive, not the caller's value.
    assert!(
        requests[0].contains("\r\nConnection: keep-alive\r\n"),
        "{requests:?}"
    );
}

/// "Connection: close" is the one exception to the restricted list.
#[test]
fn connection_close_is_allowed() {
    let (port, requests) = serve();
    assert_eq!(download(port, &[("Connection", "Close")]).unwrap(), b"ok");
    let requests = requests.lock().unwrap().clone();
    let connection: Vec<(String, String)> = header_lines(&requests[0])
        .into_iter()
        .filter(|(name, _)| name == "connection")
        .collect();
    assert_eq!(
        connection,
        vec![("connection".to_string(), "Close".to_string())]
    );
}

/// Scenario 2 of R3-1: a token read from a file with its trailing newline. Java fails with
/// "Illegal character(s) in message header value"; nothing is sent.
#[test]
fn header_value_with_a_trailing_newline_fails_as_java_does() {
    let (port, requests) = serve();
    let e = download(port, &[("Authorization", "Bearer tok\n")]).unwrap_err();
    let message = e.get_message();
    assert!(
        message.starts_with(&format!(
            "Error downloading [http://127.0.0.1:{port}/file.txt] to "
        )),
        "{e:?}"
    );
    assert!(
        message.ends_with(": Illegal character(s) in message header value: Bearer tok\n"),
        "{e:?}"
    );
    assert!(requests.lock().unwrap().is_empty());
}

#[test]
fn header_injection_through_values_and_names_fails_as_java_does() {
    let (port, requests) = serve();
    for (name, value, message) in [
        (
            "X-A",
            "v\r\nX-Injected: 1",
            "Illegal character(s) in message header value: v\r\nX-Injected: 1",
        ),
        (
            "X-A\nX-Injected",
            "v",
            "Illegal character(s) in message header field: X-A\nX-Injected",
        ),
        (
            "X-A\r\nX-Injected",
            "v",
            "Illegal character(s) in message header field: X-A\r\nX-Injected",
        ),
        (
            "X-A: v",
            "v",
            "Illegal character(s) in message header field: X-A: v",
        ),
    ] {
        let e = download(port, &[(name, value)]).unwrap_err();
        assert!(e.get_message().ends_with(message), "{e:?}");
    }
    assert!(requests.lock().unwrap().is_empty());
}

/// PORT: a bare carriage return passes the JDK's check and is written by the JDK as it is. The
/// transport refuses it (see `socket_transport::check_request_header`).
#[test]
fn bare_carriage_return_is_refused_by_the_transport() {
    let (port, requests) = serve();
    let e = download(port, &[("X-A\rX-B", "v")]).unwrap_err();
    assert!(
        e.get_message()
            .ends_with("Illegal character(s) in message header field: X-A\rX-B"),
        "{e:?}"
    );
    let e = download(port, &[("X-A", "v\rX-B: 1")]).unwrap_err();
    assert!(
        e.get_message()
            .ends_with("Illegal character(s) in message header value: v\rX-B: 1"),
        "{e:?}"
    );
    assert!(requests.lock().unwrap().is_empty());
}

// Java: sun/net/www/protocol/http/HttpURLConnection.java HttpURLConnection.checkMessageHeader
#[test]
fn check_message_header_follows_the_jdk() {
    let field = |k: &str| {
        Err(IllegalArgumentException(format!(
            "Illegal character(s) in message header field: {k}"
        )))
    };
    let value = |v: &str| {
        Err(IllegalArgumentException(format!(
            "Illegal character(s) in message header value: {v}"
        )))
    };
    assert_eq!(check_message_header("X-A", "v"), Ok(()));
    assert_eq!(check_message_header("", ""), Ok(()));
    assert_eq!(check_message_header("X\nA", "v"), field("X\nA"));
    assert_eq!(check_message_header("X:A", "v"), field("X:A"));
    // The name is checked first.
    assert_eq!(check_message_header("X:A", "v\n"), field("X:A"));
    assert_eq!(check_message_header("X-A", "v\n"), value("v\n"));
    assert_eq!(check_message_header("X-A", "\n"), value("\n"));
    assert_eq!(check_message_header("X-A", "a\nb"), value("a\nb"));
    // A line feed followed by a space or a tab is a continuation line.
    assert_eq!(check_message_header("X-A", "a\n b"), Ok(()));
    assert_eq!(check_message_header("X-A", "a\n\tb\n c"), Ok(()));
    assert_eq!(check_message_header("X-A", "a\n b\nc"), value("a\n b\nc"));
    assert_eq!(check_message_header("X-A", "a\n b\n"), value("a\n b\n"));
    // A carriage return alone is not checked by the JDK.
    assert_eq!(check_message_header("X\rA", "a\rb"), Ok(()));
}

// Java: sun/net/www/protocol/http/HttpURLConnection.java HttpURLConnection.isRestrictedHeader
#[test]
fn is_restricted_header_follows_the_jdk() {
    for name in [
        "Access-Control-Request-Headers",
        "Access-Control-Request-Method",
        "Connection",
        "Content-Length",
        "Content-Transfer-Encoding",
        "Host",
        "Keep-Alive",
        "Origin",
        "Trailer",
        "Transfer-Encoding",
        "Upgrade",
        "Via",
        "HOST",
        "host",
        "Sec-WebSocket-Key",
        "sec-",
    ] {
        assert!(is_restricted_header(name, "x"), "{name}");
    }
    assert!(!is_restricted_header("Connection", "close"));
    assert!(!is_restricted_header("connection", "CLOSE"));
    assert!(is_restricted_header("Connection", "close, upgrade"));
    // The names the JDK's list leaves commented out.
    for name in [
        "Accept-Charset",
        "Accept-Encoding",
        "Cookie",
        "Date",
        "Expect",
        "Referer",
        "TE",
        "User-Agent",
        "Authorization",
        "Proxy-Authorization",
        "Range",
        "Security",
    ] {
        assert!(!is_restricted_header(name, "x"), "{name}");
    }
}

// Java: sun/net/www/protocol/http/HttpURLConnection.java HttpURLConnection.setRequestProperty / addRequestProperty
#[test]
fn request_properties_set_and_add() {
    let mut properties = RequestProperties::default();
    properties.add_request_property("X-A", "1").unwrap();
    properties.add_request_property("x-a", "2").unwrap();
    properties.add_request_property("Host", "other").unwrap();
    properties.set_request_property("X-B", "1").unwrap();
    properties.set_request_property("x-b", "2").unwrap();
    // MessageHeader.set replaces the first property of that name.
    properties.set_request_property("X-A", "3").unwrap();
    assert_eq!(properties.get_request_property("x-a"), Some("2"));
    assert_eq!(properties.get_request_property("Host"), None);
    assert_eq!(
        properties.add_request_property("X-C", "v\n"),
        Err(IllegalArgumentException(
            "Illegal character(s) in message header value: v\n".to_string()
        ))
    );
    let lines: Vec<(String, String)> = properties.into_header_lines();
    assert_eq!(
        lines,
        vec![
            ("X-A".to_string(), "3".to_string()),
            ("x-a".to_string(), "2".to_string()),
            ("X-B".to_string(), "2".to_string()),
        ]
    );
}
