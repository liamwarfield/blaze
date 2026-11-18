//! Port of `HttpConnector`, plus the `URLConnection` / `HttpURLConnection` behaviour it relies
//! on.
//!
//! PORT: Java opens connections with `HttpURLConnection` (redirects disabled). Blaze sends
//! requests through an [`HttpTransport`]; the default is [`UreqTransport`] (blocking, rustls).
//! Transport failures are mapped to the Java exception classes HttpConnector distinguishes
//! (`UnknownHostException`, `SocketTimeoutException`, `SSLException`, ...). Their messages come
//! from the Rust stack, not the JDK.

use std::fs::File;
use std::sync::Arc;
use std::time::Duration;

use blaze_events::Event;
use blaze_events::handler::EventHandler;

use super::download_progress_event::format_grouped;
use super::http_utils;
use super::proxy_helper::{ProxyHelper, ProxyInfo, ProxyType};
use super::streams::{InputStream, ReadInputStream};
use crate::authandtls::Headers;
use crate::errors::{IoException, IoExceptionKind};
use crate::uri::JavaUri;

/// An open connection with its response (Java `URLConnection`).
pub struct UrlConnection {
    url: JavaUri,
    request_method: String,
    response_code: i32,
    response_message: Option<String>,
    headers: Vec<(String, String)>,
    body: Option<Result<Box<dyn InputStream>, IoException>>,
}

impl UrlConnection {
    /// A connection with a received response.
    pub fn new(
        url: JavaUri,
        response_code: i32,
        response_message: Option<String>,
        headers: Vec<(String, String)>,
        body: Result<Box<dyn InputStream>, IoException>,
    ) -> Self {
        UrlConnection {
            url,
            request_method: "GET".to_string(),
            response_code,
            response_message,
            headers,
            body: Some(body),
        }
    }

    /// `URLConnection.getURL()` as a URI (`HttpUtils.toUri`).
    pub fn get_url(&self) -> &JavaUri {
        &self.url
    }

    pub fn get_request_method(&self) -> &str {
        &self.request_method
    }

    pub fn get_response_code(&self) -> i32 {
        self.response_code
    }

    pub fn get_response_message(&self) -> Option<&str> {
        self.response_message.as_deref()
    }

    /// `getHeaderField(name)`: case-insensitive; the last value wins (as `MessageHeader`).
    pub fn get_header_field(&self, name: &str) -> Option<String> {
        self.headers
            .iter()
            .rev()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.clone())
    }

    pub fn get_content_encoding(&self) -> Option<String> {
        self.get_header_field("content-encoding")
    }

    /// `getInputStream()` (only once).
    pub fn get_input_stream(&mut self) -> Result<Box<dyn InputStream>, IoException> {
        match self.body.take() {
            Some(b) => b,
            None => Err(IoException::new("Stream closed")),
        }
    }

    /// `disconnect()`.
    pub fn disconnect(&mut self) {
        self.body = None;
    }
}

/// Sends one HTTP GET (no redirect following) and returns the response, or a transport error
/// mapped to a Java exception class.
pub trait HttpTransport: Send + Sync {
    fn open(
        &self,
        url: &JavaUri,
        proxy: &ProxyInfo,
        headers: &[(String, String)],
        connect_timeout_ms: i32,
        read_timeout_ms: i32,
    ) -> Result<UrlConnection, IoException>;
}

/// `Sleeper`.
pub trait Sleeper: Send + Sync {
    fn sleep_millis(&self, millis: i64);
}

/// `JavaSleeper`.
pub struct JavaSleeper;

impl Sleeper for JavaSleeper {
    fn sleep_millis(&self, millis: i64) {
        if millis > 0 {
            std::thread::sleep(Duration::from_millis(millis as u64));
        }
    }
}

/// The default transport, built on `ureq` 2 (rustls, webpki roots).
pub struct UreqTransport;

fn map_transport_error(url: &JavaUri, t: &ureq::Transport) -> IoException {
    let message = t.to_string();
    let io_kind = std::error::Error::source(t)
        .and_then(|s| s.downcast_ref::<std::io::Error>())
        .map(|e| e.kind());
    match t.kind() {
        ureq::ErrorKind::Dns => IoException::of(
            IoExceptionKind::UnknownHostException,
            url.get_host().unwrap_or("").to_string(),
        ),
        ureq::ErrorKind::InvalidUrl | ureq::ErrorKind::UnknownScheme => {
            IoException::of(IoExceptionKind::UnrecoverableHttpException, message)
        }
        _ => match io_kind {
            Some(std::io::ErrorKind::ConnectionRefused) => {
                IoException::of(IoExceptionKind::ConnectException, "Connection refused")
            }
            Some(std::io::ErrorKind::TimedOut) | Some(std::io::ErrorKind::WouldBlock) => {
                IoException::of(
                    IoExceptionKind::SocketTimeoutException,
                    if t.kind() == ureq::ErrorKind::ConnectionFailed {
                        "Connect timed out"
                    } else {
                        "Read timed out"
                    },
                )
            }
            Some(std::io::ErrorKind::ConnectionReset) => {
                IoException::of(IoExceptionKind::SocketException, "Connection reset")
            }
            _ if message.contains("tls") || message.contains("certificate") => {
                IoException::of(IoExceptionKind::SSLException, message)
            }
            _ => IoException::new(message),
        },
    }
}

impl HttpTransport for UreqTransport {
    fn open(
        &self,
        url: &JavaUri,
        proxy: &ProxyInfo,
        headers: &[(String, String)],
        connect_timeout_ms: i32,
        read_timeout_ms: i32,
    ) -> Result<UrlConnection, IoException> {
        let mut builder = ureq::AgentBuilder::new()
            .redirects(0)
            .timeout_connect(Duration::from_millis(connect_timeout_ms.max(1) as u64))
            .timeout_read(Duration::from_millis(read_timeout_ms.max(1) as u64));
        if proxy.proxy_type != ProxyType::DIRECT {
            let scheme = match proxy.proxy_type {
                ProxyType::SOCKS => "socks5",
                _ => "http",
            };
            let creds = match (proxy.username(), proxy.password()) {
                (Some(u), Some(p)) => format!("{}:{}@", u, p),
                _ => String::new(),
            };
            let spec = format!("{}://{}{}", scheme, creds, proxy.address().unwrap());
            let p = ureq::Proxy::new(&spec).map_err(|e| IoException::new(e.to_string()))?;
            builder = builder.proxy(p);
        }
        let agent = builder.build();
        let mut request = agent.request("GET", url.as_str());
        // PORT: HttpURLConnection sends repeated request properties as separate header lines;
        // ureq keeps one line per name, so repeated values are joined with ", " (equivalent per
        // RFC 9110 §5.3).
        let mut merged: Vec<(String, String)> = Vec::new();
        for (k, v) in headers {
            match merged.iter_mut().find(|(mk, _)| mk.eq_ignore_ascii_case(k)) {
                Some((_, mv)) => {
                    mv.push_str(", ");
                    mv.push_str(v);
                }
                None => merged.push((k.clone(), v.clone())),
            }
        }
        for (k, v) in &merged {
            request = request.set(k, v);
        }
        let response = match request.call() {
            Ok(r) => r,
            Err(ureq::Error::Status(_, r)) => r,
            Err(ureq::Error::Transport(t)) => return Err(map_transport_error(url, &t)),
        };
        let code = response.status() as i32;
        let message = response.status_text().to_string();
        let mut resp_headers = Vec::new();
        for name in response.headers_names() {
            for v in response.all(&name) {
                resp_headers.push((name.clone(), v.to_string()));
            }
        }
        let reader = response.into_reader();
        Ok(UrlConnection::new(
            url.clone(),
            code,
            Some(message),
            resp_headers,
            Ok(Box::new(ReadInputStream::new(reader))),
        ))
    }
}

/// A `file:` URL connection (`FileURLConnection`): `Content-Length` is the file size and the
/// body is the file; a missing file fails on `getInputStream` with `FileNotFoundException`.
fn open_file_connection(url: &JavaUri) -> UrlConnection {
    let path = url.get_path().unwrap_or_default();
    let mut headers = Vec::new();
    let body: Result<Box<dyn InputStream>, IoException> = match File::open(&path) {
        Ok(f) => {
            if let Ok(m) = f.metadata() {
                headers.push(("content-length".to_string(), m.len().to_string()));
            }
            Ok(Box::new(ReadInputStream::new(f)) as Box<dyn InputStream>)
        }
        Err(e) => Err(IoException::of(
            IoExceptionKind::FileNotFoundException,
            format!(
                "{} ({})",
                path,
                crate::decompressor::compressed_tar::strerror(&e)
            ),
        )),
    };
    UrlConnection::new(url.clone(), 200, Some("OK".to_string()), headers, body)
}

const MAX_ATTEMPTS: i32 = 8;
const MAX_REDIRECTS: i32 = 40;
const MIN_RETRY_DELAY_MS: i32 = 100;
const MIN_CONNECT_TIMEOUT_MS: i32 = 1000;
const MAX_CONNECT_TIMEOUT_MS: i32 = 10000;
const READ_TIMEOUT_MS: i32 = 20000;
const COMPRESSED_EXTENSIONS: [&str; 7] = ["bz2", "gz", "jar", "tgz", "war", "xz", "zip"];

/// `"bazel/" + BlazeVersionInfo.getVersion()`.
pub fn user_agent_value() -> String {
    format!("bazel/{}", crate::BAZEL_VERSION)
}

/// Headers to send for a URL (Java `Function<URI, ImmutableMap<String, List<String>>>`).
pub type HeaderFunction = Arc<dyn Fn(&JavaUri) -> Headers + Send + Sync>;

/// Class for establishing connections to HTTP servers for downloading files.
// Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpConnector.java:47 HttpConnector
pub struct HttpConnector {
    event_handler: Arc<dyn EventHandler>,
    proxy_helper: ProxyHelper,
    sleeper: Arc<dyn Sleeper>,
    timeout_scaling: f32,
    max_attempts: i32,
    max_retry_timeout: Duration,
    transport: Arc<dyn HttpTransport>,
}

/// `Math.random()`.
fn java_random() -> f64 {
    (uuid::Uuid::new_v4().as_u128() >> 75) as f64 / (1u64 << 53) as f64
}

impl HttpConnector {
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpConnector.java:71 HttpConnector.<init>
    pub fn new(
        event_handler: Arc<dyn EventHandler>,
        proxy_helper: ProxyHelper,
        sleeper: Arc<dyn Sleeper>,
        timeout_scaling: f32,
        max_attempts: i32,
        max_retry_timeout: Duration,
        transport: Arc<dyn HttpTransport>,
    ) -> Self {
        HttpConnector {
            event_handler,
            proxy_helper,
            sleeper,
            timeout_scaling,
            max_attempts: if max_attempts > 0 {
                max_attempts
            } else {
                MAX_ATTEMPTS
            },
            max_retry_timeout,
            transport,
        }
    }

    fn scale(&self, unscaled: i32) -> i32 {
        (unscaled as f32 * self.timeout_scaling).round() as i32
    }

    fn progress(&self, message: String) {
        self.event_handler.handle(&Event::progress(message));
    }

    // Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpConnector.java:318 HttpConnector.describeHttpResponse
    fn describe_http_response(connection: &UrlConnection) -> String {
        format!(
            "{} returned {} {}",
            connection.get_request_method(),
            connection.get_response_code(),
            connection.get_response_message().unwrap_or("")
        )
    }

    /// Connects to `original_url`, following redirects and retrying transient errors.
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpConnector.java:107 HttpConnector.connect
    pub fn connect(
        &self,
        original_url: &JavaUri,
        request_headers: &HeaderFunction,
    ) -> Result<UrlConnection, IoException> {
        let mut original_url = original_url.clone();
        let mut url = original_url.clone();
        if http_utils::is_protocol(&url, "file") {
            return Ok(open_file_connection(&url));
        }
        let mut suppressions: Vec<IoException> = Vec::new();
        let mut retries = 0;
        let mut redirects = 0;
        let mut connect_timeout = self.scale(MIN_CONNECT_TIMEOUT_MS);
        loop {
            let attempt: Result<Option<UrlConnection>, IoException> = (|| {
                let proxy_info = self.proxy_helper.create_proxy_if_needed(&url)?;
                let mut header_lines: Vec<(String, String)> = Vec::new();
                // For HTTP connections through authenticated proxies, set the
                // Proxy-Authorization header.
                if proxy_info.has_credentials() && http_utils::is_protocol(&url, "http") {
                    header_lines.push((
                        "Proxy-Authorization".to_string(),
                        proxy_info.get_proxy_authorization_header().unwrap(),
                    ));
                }
                let url_path = url.get_path().unwrap_or_default();
                let original_path = original_url.get_path().unwrap_or_default();
                let is_already_compressed = COMPRESSED_EXTENSIONS
                    .contains(&http_utils::get_extension(&url_path).as_str())
                    || COMPRESSED_EXTENSIONS
                        .contains(&http_utils::get_extension(&original_path).as_str());
                for (key, values) in request_headers(&url) {
                    if is_already_compressed && key.eq_ignore_ascii_case("Accept-Encoding") {
                        // We're not going to ask for compression if we're downloading a file
                        // that already appears to be compressed.
                        continue;
                    }
                    for value in values {
                        header_lines.push((key.clone(), value));
                    }
                }
                if !header_lines
                    .iter()
                    .any(|(k, _)| k.eq_ignore_ascii_case("User-Agent"))
                {
                    header_lines.push(("User-Agent".to_string(), user_agent_value()));
                }
                // The read timeout is always large because it stays in effect after this
                // method.
                let mut connection = match self.transport.open(
                    &url,
                    &proxy_info,
                    &header_lines,
                    connect_timeout,
                    self.scale(READ_TIMEOUT_MS),
                ) {
                    Ok(c) => c,
                    Err(e) if e.kind == IoExceptionKind::SSLException => {
                        // Check if the exception is due to a permanent error, such as a
                        // certificate validation issue.
                        let m = e.get_message();
                        if m.contains("certificate") || m.contains("CertPathValidatorException") {
                            let message = format!("TLS error: {}", m);
                            self.progress(message.clone());
                            let mut ex = IoException::of(
                                IoExceptionKind::UnrecoverableHttpException,
                                message,
                            );
                            ex.add_suppressed(e);
                            return Err(ex);
                        }
                        return Err(e);
                    }
                    Err(e) if e.kind == IoExceptionKind::UnknownHostException => {
                        let message = format!("Unknown host: {}", e.get_message());
                        self.progress(message.clone());
                        let mut ex =
                            IoException::of(IoExceptionKind::UnrecoverableHttpException, message);
                        ex.add_suppressed(e);
                        return Err(ex);
                    }
                    Err(e) => return Err(e),
                };
                let code = connection.get_response_code();
                // 206 means partial content and only happens if caller specified Range.
                if code == 200 || code == 206 {
                    Ok(Some(connection))
                } else if code == 301 || code == 302 || code == 303 || code == 307 {
                    read_all_bytes_and_close(&mut connection);
                    redirects += 1;
                    if redirects == MAX_REDIRECTS {
                        self.progress(format!("Redirect loop detected in {}", original_url));
                        return Err(IoException::of(
                            IoExceptionKind::UnrecoverableHttpException,
                            "Redirect loop detected",
                        ));
                    }
                    url = http_utils::get_location(
                        connection.get_header_field("Location").as_deref(),
                        connection.get_url(),
                    )?;
                    if code == 301 {
                        original_url = url.clone();
                    }
                    Ok(None)
                } else if code == 403 || code == 408 || code == 429 {
                    // 403: BitBucket + Amazon AWS downloads frequently flake with this code.
                    // 408: the client MAY repeat that request on a new connection.
                    // 429: could result from Bazel temporarily overloading the server.
                    Err(IoException::new(Self::describe_http_response(&connection)))
                } else if code < 500 || code == 501 || code == 505 {
                    // This is a permanent error so we're not going to retry.
                    read_all_bytes_and_close(&mut connection);
                    if code == 404 || code == 410 {
                        return Err(IoException::of(
                            IoExceptionKind::FileNotFoundException,
                            Self::describe_http_response(&connection),
                        ));
                    }
                    Err(IoException::of(
                        IoExceptionKind::UnrecoverableHttpException,
                        Self::describe_http_response(&connection),
                    ))
                } else {
                    // However we will retry on some 5xx errors, particularly 500, 502 and 503.
                    Err(IoException::new(Self::describe_http_response(&connection)))
                }
            })();
            let mut e = match attempt {
                Ok(Some(c)) => return Ok(c),
                Ok(None) => continue,
                Err(e)
                    if e.kind == IoExceptionKind::UnrecoverableHttpException
                        || e.kind == IoExceptionKind::FileNotFoundException =>
                {
                    return Err(e);
                }
                Err(e) => e,
            };
            // We don't respect the Retry-After header because it's rarely used and tends to be
            // too conservative when it is. We use exponential backoff with jitter.
            let mut raw_timeout = (MIN_RETRY_DELAY_MS as f64) * 2f64.powi(retries);
            if !self.max_retry_timeout.is_zero() {
                raw_timeout = raw_timeout.min(self.max_retry_timeout.as_millis() as f64);
            }
            let mut timeout = ((0.75 + java_random() / 2.0) * raw_timeout) as i64;
            if e.kind == IoExceptionKind::SocketTimeoutException {
                self.progress(format!("Timeout connecting to {}", url));
                connect_timeout = (connect_timeout * 2).min(self.scale(MAX_CONNECT_TIMEOUT_MS));
                // If we got connect timeout, we're already doing exponential backoff, so no
                // point in sleeping too.
                timeout = 1;
            } else if e.kind == IoExceptionKind::InterruptedIOException {
                return Err(e);
            }
            retries += 1;
            if retries == self.max_attempts {
                if e.kind == IoExceptionKind::SocketTimeoutException {
                    // Not an external interruption, but simply a failed download due to some
                    // server timing out. So rethrow it as an ordinary IOException.
                    e = IoException {
                        kind: IoExceptionKind::IOException,
                        message: e.message.clone(),
                        suppressed: Vec::new(),
                    };
                } else {
                    self.progress(format!("Error connecting to {}: {}", url, e.get_message()));
                }
                for s in suppressions {
                    e.add_suppressed(s);
                }
                return Err(e);
            }
            suppressions.push(e);
            self.progress(format!(
                "Failed to connect to {} trying again in {}ms",
                url,
                format_grouped(timeout)
            ));
            url = original_url.clone();
            self.sleeper.sleep_millis(timeout);
        }
    }
}

/// Exhausts all bytes in an HTTP response.
fn read_all_bytes_and_close(connection: &mut UrlConnection) {
    if let Ok(mut s) = connection.get_input_stream() {
        let mut buf = [0u8; 8192];
        while let Ok(n) = s.read(&mut buf) {
            if n == 0 {
                break;
            }
        }
        let _ = s.close();
    }
}

/// Default transport handle.
pub fn default_transport() -> Arc<dyn HttpTransport> {
    Arc::new(UreqTransport)
}
