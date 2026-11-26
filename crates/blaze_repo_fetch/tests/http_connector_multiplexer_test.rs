//! Port of `HttpConnectorMultiplexerTest`.

mod downloader_support;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use blaze_repo_fetch::authandtls::{Headers, StaticCredentials};
use blaze_repo_fetch::downloader::http_connector::{HttpConnector, HttpTransport, UrlConnection};
use blaze_repo_fetch::downloader::http_connector_multiplexer::{
    HttpConnectorMultiplexer, get_header_function,
};
use blaze_repo_fetch::downloader::http_stream::HttpStreamFactory;
use blaze_repo_fetch::downloader::proxy_helper::{ProxyHelper, ProxyInfo};
use blaze_repo_fetch::errors::IoException;
use blaze_repo_fetch::uri::JavaUri;
use downloader_support::{ManualClock, ManualSleeper, RecordingHandler};
use parking_lot::Mutex;

const TEST_URL: &str = "http://test.example";

/// Stands in for the mocked `HttpConnector.connect`: a transport that fails with "oops".
struct FailingTransport {
    calls: Mutex<usize>,
}

impl HttpTransport for FailingTransport {
    fn open(
        &self,
        _url: &JavaUri,
        _proxy: &ProxyInfo,
        _headers: &[(String, String)],
        _connect_timeout_ms: i32,
        _read_timeout_ms: i32,
    ) -> Result<UrlConnection, IoException> {
        *self.calls.lock() += 1;
        Err(IoException::new("oops"))
    }
}

fn multiplexer(transport: Arc<dyn HttpTransport>) -> HttpConnectorMultiplexer {
    let handler = RecordingHandler::new();
    let clock = ManualClock::new();
    let connector = HttpConnector::new(
        handler.clone(),
        ProxyHelper::new(HashMap::new()),
        ManualSleeper::new(clock.clone()),
        1.0,
        0,
        Duration::ZERO,
        transport,
    );
    HttpConnectorMultiplexer::new(
        handler.clone(),
        Arc::new(connector),
        HttpStreamFactory {
            clock,
            event_handler: handler,
        },
    )
}

fn headers(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), vec![v.to_string()]))
        .collect()
}

#[test]
#[should_panic(expected = "IllegalArgumentException")]
fn ftp_url_throws_iae() {
    let m = multiplexer(Arc::new(FailingTransport {
        calls: Mutex::new(0),
    }));
    let _ = m.connect(
        &JavaUri::create("ftp://lol.example").unwrap(),
        None,
        &Headers::new(),
        Arc::new(StaticCredentials::empty()),
        None,
    );
}

#[test]
#[ignore = "Blaze has no thread interruption"]
fn thread_is_interrupted_throws_ie_pronto_and_does_nothing_else() {}

#[test]
#[ignore = "needs a mocked HttpStream.Factory that ignores the checksum; Blaze's factory is concrete"]
fn success() {}

#[test]
fn failure() {
    let transport = Arc::new(FailingTransport {
        calls: Mutex::new(0),
    });
    let m = multiplexer(transport.clone());
    let e = m
        .connect(
            &JavaUri::create(TEST_URL).unwrap(),
            None,
            &Headers::new(),
            Arc::new(StaticCredentials::empty()),
            None,
        )
        .err()
        .unwrap();
    assert!(e.get_message().contains("oops"), "{}", e);
}

fn sorted(h: Headers) -> Vec<(String, Vec<String>)> {
    let mut v: Vec<_> = h.into_iter().collect();
    v.sort();
    v
}

#[test]
fn test_header_computation_function() {
    let handler = RecordingHandler::new();
    let base_headers = headers(&[("Accept-Encoding", "gzip"), ("User-Agent", "Bazel/testing")]);
    let mut additional: HashMap<JavaUri, Headers> = HashMap::new();
    additional.insert(
        JavaUri::create("http://hosting.example.com/user/foo/file.txt").unwrap(),
        headers(&[("Authentication", "Zm9vOmZvb3NlY3JldA==")]),
    );

    let header_function = get_header_function(
        base_headers.clone(),
        Arc::new(StaticCredentials::new(additional.clone())),
        handler.clone(),
    );
    let apply = |f: &blaze_repo_fetch::downloader::http_connector::HeaderFunction, u: &str| {
        sorted(f(&JavaUri::create(u).unwrap()).unwrap())
    };

    // Unrelated URL
    assert_eq!(
        apply(&header_function, "http://example.org/some/path/file.txt"),
        sorted(base_headers.clone())
    );
    // With auth headers
    assert_eq!(
        apply(
            &header_function,
            "http://hosting.example.com/user/foo/file.txt"
        ),
        sorted(headers(&[
            ("Accept-Encoding", "gzip"),
            ("User-Agent", "Bazel/testing"),
            ("Authentication", "Zm9vOmZvb3NlY3JldA=="),
        ]))
    );
    // Other hosts
    for u in [
        "http://hosting2.example.com/user/foo/file.txt",
        "http://sub.hosting.example.com/user/foo/file.txt",
        "http://example.com/user/foo/file.txt",
        "http://hosting.example.com.evil.example/user/foo/file.txt",
    ] {
        assert_eq!(
            apply(&header_function, u),
            sorted(base_headers.clone()),
            "{}",
            u
        );
    }

    // Verify that URL-specific headers overwrite
    let annon_auth = headers(&[("Authentication", "YW5vbnltb3VzOmZvb0BleGFtcGxlLm9yZw==")]);
    let combined = get_header_function(
        annon_auth.clone(),
        Arc::new(StaticCredentials::new(additional)),
        handler,
    );
    assert_eq!(
        apply(&combined, "http://hosting.example.com/user/foo/file.txt"),
        sorted(headers(&[("Authentication", "Zm9vOmZvb3NlY3JldA==")]))
    );
    assert_eq!(
        apply(&combined, "http://unreleated.example.org/user/foo/file.txt"),
        sorted(annon_auth)
    );
}
