//! Port of `HttpConnectorTest` (local `TcpListener` servers stand in for `ServerSocket`).

mod downloader_support;

use std::collections::HashMap;
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::thread;
use std::time::Duration;

use blaze_events::ui::Clock;
use blaze_repo_fetch::authandtls::Headers;
use blaze_repo_fetch::downloader::http_connector::{
    HeaderFunction, HttpConnector, UrlConnection, default_transport,
};
use blaze_repo_fetch::downloader::proxy_helper::ProxyHelper;
use blaze_repo_fetch::downloader::streams::transfer_to;
use blaze_repo_fetch::errors::{IoException, IoExceptionKind};
use blaze_repo_fetch::uri::JavaUri;
use downloader_support::{
    ManualClock, ManualSleeper, RecordingHandler, read_http_request, send_lines,
};
use parking_lot::Mutex;

type HeaderMap = Arc<Mutex<HashMap<String, Vec<String>>>>;

struct Fixture {
    clock: Arc<ManualClock>,
    sleeper: Arc<ManualSleeper>,
    connector: HttpConnector,
}

/// Scale timeouts down to make tests fast.
const TIMEOUT_SCALING: f32 = 0.05;

fn fixture() -> Fixture {
    let clock = ManualClock::new();
    let sleeper = ManualSleeper::new(clock.clone());
    let connector = HttpConnector::new(
        RecordingHandler::new(),
        ProxyHelper::new(HashMap::new()),
        sleeper.clone(),
        TIMEOUT_SCALING,
        0,
        Duration::ZERO,
        default_transport(),
    );
    Fixture {
        clock,
        sleeper,
        connector,
    }
}

fn no_headers() -> HeaderFunction {
    Arc::new(|_| Headers::new())
}

fn server() -> (TcpListener, u16) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    (l, port)
}

/// Accepts one connection, records the request headers and sends `lines`.
fn serve_once(listener: &TcpListener, headers: Option<&HeaderMap>, lines: &[&str]) {
    let (mut socket, _) = listener.accept().unwrap();
    let h = read_http_request(&mut socket).unwrap();
    if let Some(m) = headers {
        m.lock().extend(h);
    }
    send_lines(&mut socket, lines);
}

fn read_body(mut c: UrlConnection) -> Vec<u8> {
    let mut s = c.get_input_stream().unwrap();
    let mut out = Vec::new();
    transfer_to(&mut *s, &mut out).unwrap();
    s.close().unwrap();
    out
}

fn connect(f: &Fixture, url: &str, headers: &HeaderFunction) -> Result<UrlConnection, IoException> {
    f.connector.connect(&JavaUri::create(url).unwrap(), headers)
}

fn hello_lines() -> Vec<&'static str> {
    vec![
        "HTTP/1.1 200 OK",
        "Date: Fri, 31 Dec 1999 23:59:59 GMT",
        "Connection: close",
        "Content-Type: text/plain",
        "Content-Length: 5",
        "",
        "hello",
    ]
}

#[test]
fn local_file_download() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("file");
    std::fs::write(&path, b"this is a test").unwrap();
    let f = fixture();
    let c = connect(&f, &format!("file:{}", path.display()), &no_headers()).unwrap();
    assert_eq!(read_body(c), b"this is a test");
}

#[test]
fn bad_host_throws_io_exception() {
    let f = fixture();
    let e = connect(&f, "http://bad.example", &no_headers())
        .err()
        .unwrap();
    assert!(
        e.get_message().contains("Unknown host: bad.example"),
        "{}",
        e
    );
}

#[test]
fn normal_request() {
    let headers: HeaderMap = Arc::default();
    let (listener, port) = server();
    let h = headers.clone();
    let t = thread::spawn(move || serve_once(&listener, Some(&h), &hello_lines()));
    let f = fixture();
    let hf: HeaderFunction = Arc::new(|_| {
        let mut m = Headers::new();
        m.insert("Content-Encoding".to_string(), vec!["gzip".to_string()]);
        m
    });
    let c = connect(&f, &format!("http://localhost:{}/boo", port), &hf).unwrap();
    assert_eq!(read_body(c), b"hello");
    t.join().unwrap();
    let headers = headers.lock();
    assert_eq!(headers.get("x-method"), Some(&vec!["GET".to_string()]));
    assert_eq!(
        headers.get("x-request-uri"),
        Some(&vec!["/boo".to_string()])
    );
    assert_eq!(
        headers.get("content-encoding"),
        Some(&vec!["gzip".to_string()])
    );
}

#[test]
fn server_error_retries_connect() {
    let (listener, port) = server();
    let t = thread::spawn(move || {
        serve_once(
            &listener,
            None,
            &[
                "HTTP/1.1 500 Incredible Catastrophe",
                "Date: Fri, 31 Dec 1999 23:59:59 GMT",
                "Connection: close",
                "Content-Type: text/plain",
                "Content-Length: 8",
                "",
                "nononono",
            ],
        );
        serve_once(&listener, None, &hello_lines());
    });
    let f = fixture();
    let c = connect(&f, &format!("http://localhost:{}", port), &no_headers()).unwrap();
    assert_eq!(read_body(c), b"hello");
    assert!(f.clock.current_time_millis() > 50);
    assert!(f.clock.current_time_millis() < 150);
    t.join().unwrap();
}

#[test]
fn connection_refused_retries() {
    // Start and immediately stop server socket to get a free port.
    let port = server().1;
    let f = fixture();
    // Schedule server socket to be started only after retry to simulate connection retry.
    f.sleeper.schedule_runnable(
        move || {
            let listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
            thread::spawn(move || serve_once(&listener, None, &hello_lines()));
        },
        1,
    );
    let c = connect(&f, &format!("http://localhost:{}", port), &no_headers()).unwrap();
    assert_eq!(read_body(c), b"hello");
}

#[test]
#[ignore = "Deactivated in Bazel due to https://github.com/bazelbuild/bazel/issues/9380 (@Ignore)"]
fn socket_timeout_retries() {}

/// It is an important part of the HttpConnector contract to not throw a raw
/// SocketTimeoutException.
#[test]
fn socket_timeout_throws_io_exception_instead_of_socket_timeout_exception() {
    let (listener, port) = server();
    let _t = thread::spawn(move || {
        // Do nothing to cause SocketTimeoutException on client side.
        let (socket, _) = listener.accept().unwrap();
        drop(socket);
        // Keep the listener open (unaccepted connections then time out on read).
        thread::sleep(Duration::from_secs(60));
    });
    let f = fixture();
    let e = connect(&f, &format!("http://localhost:{}", port), &no_headers())
        .err()
        .unwrap();
    assert_ne!(e.kind, IoExceptionKind::SocketTimeoutException);
    // PORT: Java keeps the SocketTimeoutException as the cause; Blaze keeps its message.
    let timed_out = |x: &IoException| x.get_message().to_lowercase().contains("timed out");
    assert!(
        timed_out(&e)
            || e.suppressed
                .iter()
                .any(|s| s.kind == IoExceptionKind::SocketTimeoutException && timed_out(s)),
        "{:?}",
        e
    );
}

#[test]
fn permanent_error_does_not_retry_and_throws_io_exception() {
    let (listener, port) = server();
    let t = thread::spawn(move || {
        serve_once(
            &listener,
            None,
            &[
                "HTTP/1.1 401 Unauthorized",
                "Date: Fri, 31 Dec 1999 23:59:59 GMT",
                "Connection: close",
                "Content-Type: text/plain",
                "Content-Length: 0",
                "",
                "",
            ],
        )
    });
    let f = fixture();
    let e = connect(&f, &format!("http://localhost:{}", port), &no_headers())
        .err()
        .unwrap();
    assert!(e.get_message().contains("401 Unauthorized"), "{}", e);
    t.join().unwrap();
}

#[test]
fn permanent_error_not_found_does_not_retry_and_throws_file_not_found_exception() {
    let (listener, port) = server();
    let t = thread::spawn(move || {
        serve_once(
            &listener,
            None,
            &[
                "HTTP/1.1 404 Not Found",
                "Date: Fri, 31 Dec 1999 23:59:59 GMT",
                "Connection: close",
                "Content-Type: text/plain",
                "Content-Length: 0",
                "",
                "",
            ],
        )
    });
    let f = fixture();
    let e = connect(&f, &format!("http://localhost:{}", port), &no_headers())
        .err()
        .unwrap();
    assert_eq!(e.kind, IoExceptionKind::FileNotFoundException);
    assert!(e.get_message().contains("404 Not Found"), "{}", e);
    t.join().unwrap();
}

#[test]
fn permanent_error_consumes_payload_before_returningn() {
    let consumed = Arc::new(AtomicBool::new(false));
    let (listener, port) = server();
    let c2 = consumed.clone();
    let t = thread::spawn(move || {
        serve_once(
            &listener,
            None,
            &[
                "HTTP/1.1 501 Oh No",
                "Date: Fri, 31 Dec 1999 23:59:59 GMT",
                "Connection: close",
                "Content-Type: text/plain",
                "Content-Length: 1",
                "",
                "b",
            ],
        );
        c2.store(true, Ordering::SeqCst);
    });
    let f = fixture();
    assert!(connect(&f, &format!("http://localhost:{}", port), &no_headers()).is_err());
    t.join().unwrap();
    assert!(consumed.load(Ordering::SeqCst));
    assert_eq!(f.clock.current_time_millis(), 0);
}

fn always(status: &'static str, tries: Arc<AtomicI32>) -> u16 {
    let (listener, port) = server();
    thread::spawn(move || {
        loop {
            let Ok((mut socket, _)) = listener.accept() else {
                return;
            };
            if read_http_request(&mut socket).is_err() {
                continue;
            }
            send_lines(
                &mut socket,
                &[
                    status,
                    "Date: Fri, 31 Dec 1999 23:59:59 GMT",
                    "Connection: close",
                    "Content-Type: text/plain",
                    "Content-Length: 0",
                    "",
                    "",
                ],
            );
            tries.fetch_add(1, Ordering::SeqCst);
        }
    });
    port
}

#[test]
fn always500_gives_up_eventually() {
    let tries = Arc::new(AtomicI32::new(0));
    let port = always("HTTP/1.1 500 Oh My", tries.clone());
    let f = fixture();
    let e = connect(&f, &format!("http://localhost:{}", port), &no_headers())
        .err()
        .unwrap();
    assert!(e.get_message().contains("500 Oh My"), "{}", e);
    assert!(tries.load(Ordering::SeqCst) > 2);
}

#[test]
fn server_says403_client_retries_anyway() {
    let tries = Arc::new(AtomicI32::new(0));
    let port = always("HTTP/1.1 403 Forbidden", tries.clone());
    let f = fixture();
    let e = connect(&f, &format!("http://localhost:{}", port), &no_headers())
        .err()
        .unwrap();
    assert!(e.get_message().contains("403 Forbidden"), "{}", e);
    assert!(tries.load(Ordering::SeqCst) > 2);
}

#[test]
fn path_redirect_301() {
    redirect_to_different_path_works("301");
}

#[test]
fn server_redirect_301() {
    redirect_to_different_server_works("301");
}

/// Also tests behavior for 302 and 307 codes.
#[test]
fn path_redirect_303() {
    redirect_to_different_path_works("303");
}

#[test]
fn server_redirects_303() {
    redirect_to_different_server_works("303");
}

fn redirect_to_different_path_works(code: &str) {
    let redirect_code = format!("HTTP/1.1 {} Redirect", code);
    let headers1: HeaderMap = Arc::default();
    let headers2: HeaderMap = Arc::default();
    let (listener, port) = server();
    let (h1, h2) = (headers1.clone(), headers2.clone());
    let t = thread::spawn(move || {
        serve_once(
            &listener,
            Some(&h1),
            &[
                &redirect_code,
                "Date: Fri, 31 Dec 1999 23:59:59 GMT",
                "Connection: close",
                "Location: /doodle.tar.gz",
                "Content-Length: 0",
                "",
                "",
            ],
        );
        serve_once(
            &listener,
            Some(&h2),
            &[
                "HTTP/1.1 200 OK",
                "Date: Fri, 31 Dec 1999 23:59:59 GMT",
                "Connection: close",
                "Content-Type: text/plain",
                "Content-Length: 0",
                "",
                "",
            ],
        );
    });
    let f = fixture();
    let c = connect(&f, &format!("http://localhost:{}", port), &no_headers()).unwrap();
    assert_eq!(
        c.get_url(),
        &JavaUri::create(&format!("http://localhost:{}/doodle.tar.gz", port)).unwrap()
    );
    assert!(read_body(c).is_empty());
    t.join().unwrap();
    assert_eq!(
        headers1.lock().get("x-request-uri"),
        Some(&vec!["/".to_string()])
    );
    assert_eq!(
        headers2.lock().get("x-request-uri"),
        Some(&vec!["/doodle.tar.gz".to_string()])
    );
}

fn redirect_to_different_server_works(code: &str) {
    let redirect_code = format!("HTTP/1.1 {} Redirect", code);
    let basic1 = "Basic b25lOmZpcnN0c2VjcmV0";
    let basic2 = "Basic dHdvOnNlY29uZHNlY3JldA==";
    let headers1: HeaderMap = Arc::default();
    let headers2: HeaderMap = Arc::default();
    let (server1, port1) = server();
    let (server2, port2) = server();
    let h1 = headers1.clone();
    let t1 = thread::spawn(move || {
        let location = format!("Location: http://localhost:{}/doodle.tar.gz", port2);
        serve_once(
            &server1,
            Some(&h1),
            &[
                &redirect_code,
                "Date: Fri, 31 Dec 1999 23:59:59 GMT",
                "Connection: close",
                &location,
                "Content-Length: 0",
                "",
                "",
            ],
        );
    });
    let h2 = headers2.clone();
    let t2 = thread::spawn(move || serve_once(&server2, Some(&h2), &hello_lines()));
    // Header function that provides different auth headers for the two servers.
    let auth_headers: HeaderFunction = Arc::new(move |url: &JavaUri| {
        let mut m = Headers::new();
        if url.get_port() == port1 as i32 {
            m.insert("Authentication".to_string(), vec![basic1.to_string()]);
        } else if url.get_port() == port2 as i32 {
            m.insert("Authentication".to_string(), vec![basic2.to_string()]);
        }
        m
    });
    let f = fixture();
    let c = connect(&f, &format!("http://localhost:{}", port1), &auth_headers).unwrap();
    assert_eq!(
        c.get_url(),
        &JavaUri::create(&format!("http://localhost:{}/doodle.tar.gz", port2)).unwrap()
    );
    assert_eq!(read_body(c), b"hello");
    t1.join().unwrap();
    t2.join().unwrap();
    // Verify that the correct form of authentication is used for each server.
    assert_eq!(
        headers1.lock().get("authentication"),
        Some(&vec![basic1.to_string()])
    );
    assert_eq!(
        headers2.lock().get("authentication"),
        Some(&vec![basic2.to_string()])
    );
}
