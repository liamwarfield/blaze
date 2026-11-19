//! Proxy authentication against the oracle's recordings (`tests/data/proxy`, made with
//! `scripts/proxy/run oracle ...`; see the README there).
//!
//! Each test replays one scenario: a proxy on 127.0.0.1 answers the n-th request with the n-th
//! response of the scenario (the last repeats), exactly as `scripts/proxy/fake-proxy` did for
//! the oracle, and records every request head. The requests Blaze sends for a number of
//! connection objects (`HttpURLConnection`s: one per HttpConnector attempt) must equal the
//! oracle's first requests byte for byte, in order.
//!
//! Not compared: the TLS ClientHello inside a tunnel and its error text (rustls, not the JDK);
//! which connection a request goes on (the JDK reuses a keep-alive connection after a 200 through
//! the proxy, Blaze opens one per request; every request after a 407 is on a new connection in
//! both).

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use blaze_repo_fetch::downloader::http_connector::UrlConnection;
use blaze_repo_fetch::downloader::jdk_http_auth::JvmNetState;
use blaze_repo_fetch::downloader::proxy_helper::{ProxyInfo, create_proxy_info_in};
use blaze_repo_fetch::downloader::socket_transport::open_in;
use blaze_repo_fetch::errors::IoException;
use blaze_repo_fetch::uri::JavaUri;
use blaze_util::string_encoding::latin1_decode;

const SCHEMES: [(&str, &str); 6] = [
    ("basic", "Basic realm=\"blaze-proxy\""),
    (
        "basicutf8",
        "Basic realm=\"blaze-proxy\", charset=\"UTF-8\"",
    ),
    (
        "digest",
        "Digest realm=\"blaze-proxy\", nonce=\"0123456789abcdef\", qop=\"auth\", algorithm=MD5",
    ),
    (
        "digest256",
        "Digest realm=\"blaze-proxy\", nonce=\"0123456789abcdef\", qop=\"auth\", algorithm=SHA-256",
    ),
    ("negotiate", "Negotiate"),
    ("ntlm", "NTLM"),
];

fn data_dir(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/proxy")
        .join(name)
}

/// `scenario.txt`: `<url>[,<url>...] <VAR>=<proxy url with PORT> <responses>`.
struct Scenario {
    urls: Vec<String>,
    proxy_url: String,
    responses: Vec<String>,
    /// The oracle's request heads, in order (`NN-cC.req`).
    oracle_requests: Vec<Vec<u8>>,
}

fn scenario(name: &str) -> Scenario {
    let dir = data_dir(name);
    let text = std::fs::read(dir.join("scenario.txt")).unwrap();
    // The proxy URL holds UTF-8 bytes (the environment); keep them as an internal string.
    let text = latin1_decode(&text).into_owned();
    let fields: Vec<&str> = text.split_whitespace().collect();
    let mut reqs: Vec<(u32, PathBuf)> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "req"))
        .map(|p| {
            let n = p.file_name().unwrap().to_str().unwrap()
                [..p.file_name().unwrap().to_str().unwrap().find('-').unwrap()]
                .parse()
                .unwrap();
            (n, p)
        })
        .collect();
    reqs.sort();
    Scenario {
        urls: fields[0].split(',').map(str::to_string).collect(),
        proxy_url: fields[1].split_once('=').unwrap().1.to_string(),
        responses: fields[2].split(',').map(str::to_string).collect(),
        oracle_requests: reqs
            .into_iter()
            .map(|(_, p)| std::fs::read(p).unwrap())
            .collect(),
    }
}

fn response(spec: &str, connect: bool) -> (Vec<u8>, bool) {
    if spec == "ok" {
        if connect {
            return (
                b"HTTP/1.1 200 Connection established\r\n\r\n".to_vec(),
                true,
            );
        }
        return (
            b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nContent-Type: text/plain\r\n\r\nhello\n"
                .to_vec(),
            false,
        );
    }
    let (kind, schemes) = spec.split_once(':').unwrap();
    let mut out = String::from("HTTP/1.1 407 Proxy Authentication Required\r\n");
    for s in schemes.split('+') {
        let value = SCHEMES.iter().find(|(n, _)| *n == s).unwrap().1;
        out.push_str(&format!("Proxy-Authenticate: {value}\r\n"));
    }
    out.push_str("Content-Length: 0\r\n");
    if kind == "407close" {
        out.push_str("Connection: close\r\n\r\n");
    } else {
        out.push_str("Connection: keep-alive\r\nProxy-Connection: keep-alive\r\n\r\n");
    }
    (out.into_bytes(), false)
}

fn read_head(stream: &mut TcpStream) -> Option<Vec<u8>> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        match stream.read(&mut byte) {
            Ok(1) => head.push(byte[0]),
            _ => return None,
        }
    }
    Some(head)
}

/// The scripted proxy. Returns its port and the request heads it receives.
fn fake_proxy(responses: Vec<String>) -> (u16, Arc<Mutex<Vec<Vec<u8>>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = requests.clone();
    std::thread::spawn(move || {
        for socket in listener.incoming() {
            let Ok(mut client) = socket else { return };
            let recorded = recorded.clone();
            let responses = responses.clone();
            std::thread::spawn(move || {
                let _ = client.set_read_timeout(Some(Duration::from_secs(10)));
                while let Some(head) = read_head(&mut client) {
                    let n = {
                        let mut r = recorded.lock().unwrap();
                        r.push(head.clone());
                        r.len()
                    };
                    let spec = &responses[n.min(responses.len()) - 1];
                    let (bytes, tunnel) = response(spec, head.starts_with(b"CONNECT "));
                    if client.write_all(&bytes).is_err() {
                        return;
                    }
                    if tunnel {
                        // Read the ClientHello, then end the tunnel.
                        let mut buf = [0u8; 4096];
                        let _ = client.read(&mut buf);
                        let _ = client.shutdown(Shutdown::Both);
                        return;
                    }
                }
            });
        }
    });
    (port, requests)
}

/// What one connection object gave.
#[derive(Debug)]
enum Outcome {
    Response(i32, Vec<u8>),
    Error(String),
    Unported(String),
}

fn read_body(mut c: UrlConnection) -> Result<Vec<u8>, IoException> {
    let mut stream = c.get_input_stream()?;
    let mut out = Vec::new();
    let mut buf = [0u8; 64];
    loop {
        let n = stream.read(&mut buf)?;
        if n == 0 {
            return Ok(out);
        }
        out.extend_from_slice(&buf[..n]);
    }
}

/// Runs `opens` connection objects for each URL of the scenario (in a fresh JVM state, after a
/// direct connection first when `direct_first`), and checks the requests against the oracle.
fn run(name: &str, opens: usize, direct_first: bool, expected_requests: usize) -> Vec<Outcome> {
    let s = scenario(name);
    let (port, requests) = fake_proxy(s.responses.clone());
    let state = JvmNetState::new();
    if direct_first {
        // A download without a proxy made the first HttpURLConnection.
        state.http_url_connection_class_init();
    }
    let proxy_url = s.proxy_url.replace("PORT", &port.to_string());
    let mut outcomes = Vec::new();
    for url in &s.urls {
        let url = JavaUri::create(url).unwrap();
        for _ in 0..opens {
            // What HttpConnector does per attempt: ProxyHelper, then the request properties.
            let proxy: ProxyInfo =
                create_proxy_info_in(&state, Some(&proxy_url), None, None).unwrap();
            let mut headers = Vec::new();
            if proxy.has_credentials() && url.get_scheme() == Some("http") {
                headers.push((
                    "Proxy-Authorization".to_string(),
                    proxy.get_proxy_authorization_header().unwrap(),
                ));
            }
            headers.push(("Accept-Encoding".to_string(), "gzip".to_string()));
            headers.push(("User-Agent".to_string(), "Bazel/release 9.3.2".to_string()));
            let result = catch_unwind(AssertUnwindSafe(|| {
                open_in(&state, &url, &proxy, &headers, 5000, 5000)
            }));
            outcomes.push(match result {
                Ok(Ok(c)) => {
                    let code = c.get_response_code();
                    Outcome::Response(code, read_body(c).unwrap())
                }
                Ok(Err(e)) => Outcome::Error(e.get_message().to_string()),
                Err(panic) => Outcome::Unported(
                    panic
                        .downcast_ref::<String>()
                        .cloned()
                        .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
                        .unwrap_or_default(),
                ),
            });
        }
    }
    let got = requests.lock().unwrap().clone();
    let show = |r: &[Vec<u8>]| -> Vec<String> {
        r.iter()
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .collect()
    };
    assert_eq!(got.len(), expected_requests, "{:#?}", show(&got));
    assert!(s.oracle_requests.len() >= got.len());
    assert_eq!(show(&got), show(&s.oracle_requests[..got.len()]));
    outcomes
}

const TUNNEL_407: &str =
    "Unable to tunnel through proxy. Proxy returns \"HTTP/1.1 407 Proxy Authentication Required\"";

fn assert_tunnel_failed(o: &Outcome) {
    match o {
        Outcome::Error(m) => assert_eq!(m, TUNNEL_407),
        other => panic!("{other:?}"),
    }
}

fn assert_tls_after_tunnel(o: &Outcome) {
    match o {
        Outcome::Error(m) => assert_ne!(m, TUNNEL_407),
        other => panic!("{other:?}"),
    }
}

fn assert_unported(o: &Outcome, what: &str) {
    match o {
        Outcome::Unported(m) => assert!(
            m.starts_with(&format!("blaze: not ported yet: {what}")),
            "{m}"
        ),
        other => panic!("{other:?}"),
    }
}

fn assert_response(o: &Outcome, code: i32) {
    match o {
        Outcome::Response(c, body) => {
            assert_eq!(*c, code);
            if code == 200 {
                assert_eq!(body, b"hello\n");
            }
        }
        other => panic!("{other:?}"),
    }
}

// --- https through the proxy (CONNECT) -----------------------------------------------------

/// No challenge: the CONNECT carries no credentials (Blaze sent them before the fix).
#[test]
fn https_200_creds() {
    let o = run("https-200-creds", 2, false, 2);
    assert_tls_after_tunnel(&o[0]);
    assert_tls_after_tunnel(&o[1]);
}

/// Basic challenge: answered on a new connection, then cached and sent up front.
#[test]
fn https_407basic_ok() {
    let o = run("https-407basic-ok", 2, false, 3);
    assert_tls_after_tunnel(&o[0]);
    assert_tls_after_tunnel(&o[1]);
}

#[test]
fn https_407basic_close() {
    let o = run("https-407basic-close", 2, false, 3);
    assert_tls_after_tunnel(&o[0]);
}

/// Wrong password: 20 requests, then the tunnel fails; nothing was cached.
#[test]
fn https_407basic_wrong() {
    let o = run("https-407basic-wrong", 2, false, 40);
    assert_tunnel_failed(&o[0]);
}

#[test]
fn https_407basic_nocreds() {
    let o = run("https-407basic-nocreds", 2, false, 2);
    assert_tunnel_failed(&o[0]);
    assert_tunnel_failed(&o[1]);
}

/// An HttpURLConnection made before ProxyHelper saw credentials: Basic stays disabled for
/// tunnels (the oracle's recording: a direct registry fetch came first).
#[test]
fn https_407basic_after_direct() {
    let o = run("https-407basic-after-direct", 2, true, 2);
    assert_tunnel_failed(&o[0]);
    assert_tunnel_failed(&o[1]);
}

/// Cached credentials refused: they leave the cache, the Authenticator's go out until the
/// limit, and the next connection object starts without credentials.
#[test]
fn https_cache_evict() {
    let o = run("https-cache-evict", 3, false, 42);
    assert_tls_after_tunnel(&o[0]);
    assert_tunnel_failed(&o[1]);
    assert_tunnel_failed(&o[2]);
}

#[test]
fn https_enc_raw() {
    let o = run("https-enc-raw", 1, false, 2);
    assert_tls_after_tunnel(&o[0]);
}

#[test]
fn https_enc_pct() {
    let o = run("https-enc-pct", 1, false, 2);
    assert_tls_after_tunnel(&o[0]);
}

#[test]
fn https_enc_raw_utf8() {
    let o = run("https-enc-raw-utf8", 1, false, 2);
    assert_tls_after_tunnel(&o[0]);
}

/// The CONNECT `Host` has the port when it is not 443.
#[test]
fn https_port() {
    let o = run("https-port", 1, false, 2);
    assert_tls_after_tunnel(&o[0]);
}

/// The oracle sends nothing for Digest with MD5 (disabled in the JDK); Blaze does not port the
/// Digest algorithm checks and stops where the JDK decides.
#[test]
fn https_407digest() {
    let o = run("https-407digest", 1, false, 1);
    assert_unported(&o[0], "DigestAuthentication");
}

#[test]
fn https_407digest256() {
    let o = run("https-407digest256", 1, false, 1);
    assert_unported(&o[0], "DigestAuthentication");
}

#[test]
fn https_407digest256_nocreds() {
    let o = run("https-407digest256-nocreds", 2, false, 2);
    assert_tunnel_failed(&o[0]);
    assert_tunnel_failed(&o[1]);
}

#[test]
fn https_407ntlm() {
    let o = run("https-407ntlm", 1, false, 1);
    assert_unported(&o[0], "NTLMAuthentication");
}

#[test]
fn https_407ntlm_nocreds() {
    let o = run("https-407ntlm-nocreds", 2, false, 2);
    assert_tunnel_failed(&o[0]);
    assert_tunnel_failed(&o[1]);
}

/// NTLM is preferred to Basic: never a fall back to Basic.
#[test]
fn https_407ntlm_basic() {
    let o = run("https-407ntlm-basic", 1, false, 1);
    assert_unported(&o[0], "NTLMAuthentication");
}

/// Digest (MD5) is preferred to Basic; the oracle then sends nothing at all.
#[test]
fn https_407digestmd5_basic() {
    let o = run("https-407digestmd5-basic", 1, false, 1);
    assert_unported(&o[0], "DigestAuthentication");
}

/// The oracle has no Kerberos and sends nothing; whether the JDK can is not ported.
#[test]
fn https_407negotiate() {
    let o = run("https-407negotiate", 1, false, 1);
    assert_unported(&o[0], "NegotiateAuthentication.isSupported");
}

#[test]
fn https_407multi() {
    let o = run("https-407multi", 1, false, 1);
    assert_unported(&o[0], "NegotiateAuthentication.isSupported");
}

// --- http through the proxy ------------------------------------------------------------------

/// Bazel's own `Proxy-Authorization` (HttpConnector, internal bytes) goes with the first request.
#[test]
fn http_200_creds() {
    let o = run("http-200-creds", 1, false, 1);
    assert_response(&o[0], 200);
}

#[test]
fn http_200_nocreds() {
    let o = run("http-200-nocreds", 1, false, 1);
    assert_response(&o[0], 200);
}

#[test]
fn http_407basic_ok() {
    let o = run("http-407basic-ok", 1, false, 2);
    assert_response(&o[0], 200);
}

/// 20 requests, then the 407 is the response (HttpConnector: "GET returned 407 ...").
#[test]
fn http_407basic_wrong() {
    let o = run("http-407basic-wrong", 1, false, 20);
    assert_response(&o[0], 407);
}

#[test]
fn http_407basic_nocreds() {
    let o = run("http-407basic-nocreds", 1, false, 1);
    assert_response(&o[0], 407);
}

#[test]
fn http_407digest() {
    let o = run("http-407digest", 1, false, 1);
    assert_unported(&o[0], "DigestAuthentication");
}

#[test]
fn http_407digest256() {
    let o = run("http-407digest256", 1, false, 1);
    assert_unported(&o[0], "DigestAuthentication");
}

#[test]
fn http_407ntlm() {
    let o = run("http-407ntlm", 1, false, 1);
    assert_unported(&o[0], "NTLMAuthentication");
}

#[test]
fn http_407multi() {
    let o = run("http-407multi", 1, false, 1);
    assert_unported(&o[0], "NegotiateAuthentication.isSupported");
}

#[test]
fn http_enc_pct() {
    let o = run("http-enc-pct", 1, false, 1);
    assert_response(&o[0], 200);
}

#[test]
fn http_enc_raw() {
    let o = run("http-enc-raw", 1, false, 1);
    assert_response(&o[0], 200);
}

/// After a 407 the JDK's own value (ISO-8859-1 of the Unicode credentials) replaces Bazel's
/// (the internal bytes) in place.
#[test]
fn http_enc_raw_407() {
    let o = run("http-enc-raw-407", 1, false, 2);
    assert_response(&o[0], 200);
}

/// The cached answer replaces Bazel's header on the next download through the same proxy.
#[test]
fn http_enc_raw_407_twice() {
    let o = run("http-enc-raw-407-twice", 1, false, 3);
    assert_response(&o[0], 200);
    assert_response(&o[1], 200);
}

#[test]
fn http_enc_raw_utf8() {
    let o = run("http-enc-raw-utf8", 1, false, 2);
    assert_response(&o[0], 200);
}

#[test]
fn http_cache_evict() {
    let o = run("http-cache-evict", 1, false, 22);
    assert_response(&o[0], 200);
    assert_response(&o[1], 407);
}
