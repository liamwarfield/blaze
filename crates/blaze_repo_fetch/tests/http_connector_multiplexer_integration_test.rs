//! Port of `HttpConnectorMultiplexerIntegrationTest`.
//!
//! The Java test mocks `ProxyHelper` (always `NO_PROXY`) and `Sleeper` (no-op) and uses a
//! `ManualClock`; here those are a `ProxyHelper` with an empty environment, a no-op sleeper and
//! a fixed clock.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Barrier};
use std::thread;

use blaze_events::StoredEventHandler;
use blaze_events::handler::{EventHandler, ExtendedEventHandler};
use blaze_events::ui::Clock;
use blaze_repo_fetch::authandtls::{Headers, StaticCredentials};
use blaze_repo_fetch::cache::KeyType;
use blaze_repo_fetch::downloader::Checksum;
use blaze_repo_fetch::downloader::http_connector::{HttpConnector, Sleeper, default_transport};
use blaze_repo_fetch::downloader::http_connector_multiplexer::HttpConnectorMultiplexer;
use blaze_repo_fetch::downloader::http_stream::HttpStreamFactory;
use blaze_repo_fetch::downloader::proxy_helper::ProxyHelper;
use blaze_repo_fetch::downloader::streams::{InputStream, transfer_to};
use blaze_repo_fetch::errors::IoException;
use blaze_repo_fetch::uri::JavaUri;

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

fn send_lines(mut socket: &TcpStream, data: &[&str]) {
    let _ = socket.write_all(data.join("\r\n").as_bytes());
    let _ = socket.flush();
}

/// `ManualClock` (never advanced).
struct ManualClock;

impl Clock for ManualClock {
    fn current_time_millis(&self) -> i64 {
        0
    }

    fn nano_time(&self) -> i64 {
        0
    }
}

/// The mocked `Sleeper`: does nothing.
struct NoSleep;

impl Sleeper for NoSleep {
    fn sleep_millis(&self, _millis: i64) {}
}

fn hello_sha256() -> Checksum {
    Checksum::from_string(
        KeyType::SHA256,
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
    )
    .unwrap()
}

fn multiplexer() -> HttpConnectorMultiplexer {
    let stored = Arc::new(StoredEventHandler::new());
    let event_handler: Arc<dyn ExtendedEventHandler> = stored.clone();
    let plain: Arc<dyn EventHandler> = stored;
    let connector = HttpConnector::new(
        plain.clone(),
        ProxyHelper::new(Default::default()),
        Arc::new(NoSleep),
        0.15,
        0,
        std::time::Duration::ZERO,
        default_transport(),
    );
    let http_stream_factory = HttpStreamFactory {
        clock: Arc::new(ManualClock),
        event_handler,
    };
    HttpConnectorMultiplexer::new(plain, Arc::new(connector), http_stream_factory)
}

fn connect(m: &HttpConnectorMultiplexer, port: u16) -> Result<Vec<u8>, IoException> {
    let url = JavaUri::create(&format!("http://localhost:{}", port)).unwrap();
    let checksum = hello_sha256();
    let mut stream = m.connect(
        &url,
        Some(&checksum),
        &Headers::new(),
        Arc::new(StaticCredentials::empty()),
        None,
    )?;
    let mut out = Vec::new();
    transfer_to(&mut stream, &mut out)?;
    stream.close()?;
    Ok(out)
}

#[test]
fn success_with_retry() {
    let barrier = Arc::new(Barrier::new(2));
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = server.local_addr().unwrap().port();
    let b = barrier.clone();
    thread::spawn(move || {
        b.wait();
        for status in ["503 MELTDOWN", "500 ERROR", "200 OK"] {
            let (socket, _) = server.accept().unwrap();
            read_http_request(&socket);
            send_lines(
                &socket,
                &[
                    &format!("HTTP/1.1 {}", status),
                    "Date: Fri, 31 Dec 1999 23:59:59 GMT",
                    "Connection: close",
                    "",
                    "hello",
                ],
            );
        }
    });
    barrier.wait();
    assert_eq!(connect(&multiplexer(), port).unwrap(), b"hello");
}

#[test]
fn captive_portal_is_avoided() {
    let barrier = Arc::new(Barrier::new(2));
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = server.local_addr().unwrap().port();
    let b = barrier.clone();
    thread::spawn(move || {
        b.wait();
        let (socket, _) = server.accept().unwrap();
        read_http_request(&socket);
        send_lines(
            &socket,
            &[
                "HTTP/1.1 200 OK",
                "Date: Fri, 31 Dec 1999 23:59:59 GMT",
                "Connection: close",
                "",
                "Never gonna give you up etc.",
            ],
        );
    });
    barrier.wait();
    let e = connect(&multiplexer(), port).unwrap_err();
    let re = regex::Regex::new("Checksum was .+ but wanted").unwrap();
    assert!(re.is_match(&e.get_message()), "{}", e);
}

#[test]
fn retry_but_keeps_failing() {
    let barrier = Arc::new(Barrier::new(2));
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = server.local_addr().unwrap().port();
    let b = barrier.clone();
    thread::spawn(move || {
        b.wait();
        for socket in server.incoming() {
            let Ok(socket) = socket else { continue };
            read_http_request(&socket);
            send_lines(
                &socket,
                &[
                    "HTTP/1.1 503 MELTDOWN",
                    "Date: Fri, 31 Dec 1999 23:59:59 GMT",
                    "Connection: close",
                    "",
                    "",
                ],
            );
        }
    });
    barrier.wait();
    let e = connect(&multiplexer(), port).unwrap_err();
    assert!(
        e.get_message().contains("GET returned 503 MELTDOWN"),
        "{}",
        e
    );
}
