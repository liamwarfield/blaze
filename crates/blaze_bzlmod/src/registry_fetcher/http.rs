//! The slice of Bazel's HTTP downloader that `DownloadManager.downloadAndReadOneUrlForBzlmod`
//! uses: `HttpDownloader.downloadAndRead`, `HttpConnectorMultiplexer.connect`,
//! `HttpConnector.connect` (retries, redirects, error classification), `HttpStream.Factory.create`
//! (range resumption, content-length check, transparent gunzip, checksum), and the `HttpUtils` /
//! `ProxyHelper` bits they call.
//!
//! PORT: Java opens connections with `HttpURLConnection`; this port sends requests with `ureq` 2
//! (redirects disabled) and maps transport failures to the Java exception classes HttpConnector
//! distinguishes. Messages of JDK-raised exceptions (socket errors, TLS, gzip format) come from
//! the Rust stack, not the JDK.
//!
//! PORT: the payload is buffered in memory (as `downloadAndRead` does) and then checked in the
//! order the Java stream stack applies its checks at end of stream: content length, gunzip,
//! checksum. Java's `ProgressInputStream` (`DownloadProgressEvent` posts, UI only) and
//! `InterruptibleInputStream` (thread interrupts) are not ported.
//!
// PORT: move to blaze_repo_fetch — temporary stand-in for DownloadManager while blaze_repo_fetch
// cannot build next to starlark (download_cache.rs:56).

use std::io::Read;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use blaze_bzlmod_model::{Checksum, JavaUri};
use blaze_events::{Event, ExtendedEventHandler};
use indexmap::IndexMap;

use super::{IoException, IoExceptionKind, java_io_message};

/// Request headers: name to values, in insertion order (Java `Map<String, List<String>>`).
pub type Headers = IndexMap<String, Vec<String>>;

/// `BlazeVersionInfo.instance().getReleaseName()` of the pinned release (`"release " + build
/// label`).
// PORT: the build label of the pin (UPSTREAM.json); blaze_runtime's version_info holds the same
// value but is not a dependency of this crate.
pub const RELEASE_NAME: &str = "release 9.3.2";

// ---------------------------------------------------------------------------------------------
// HttpUtils
// ---------------------------------------------------------------------------------------------

/// Returns true if `uri` is supported by HttpDownloader.
// Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpUtils.java:32 HttpUtils.isUrlSupportedByDownloader
pub fn is_url_supported_by_downloader(uri: &JavaUri) -> bool {
    is_http(uri) || is_protocol(uri, "file")
}

// Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpUtils.java:36 HttpUtils.isHttp
pub fn is_http(uri: &JavaUri) -> bool {
    is_protocol(uri, "http") || is_protocol(uri, "https")
}

// Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpUtils.java:40 HttpUtils.isProtocol
pub fn is_protocol(uri: &JavaUri, protocol: &str) -> bool {
    // An implementation should accept uppercase letters as equivalent to lowercase in scheme names
    // (e.g., allow "HTTP" as well as "http") for the sake of robustness. Quoth RFC3986 § 3.1
    uri.get_scheme()
        .is_some_and(|s| s.eq_ignore_ascii_case(protocol))
}

// Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpUtils.java:53 HttpUtils.getExtension
pub fn get_extension(path: &str) -> String {
    match path.rfind('.') {
        None => String::new(),
        Some(index) => path[index + 1..].to_ascii_lowercase(),
    }
}

/// The parts of a server-based authority (`URI.getUserInfo`, `getHost`, `getPort`).
// PORT: raw (undecoded) user info; IPv6 literals keep their brackets as `URI.getHost` does.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ServerAuthority {
    user_info: Option<String>,
    host: Option<String>,
    port: i32,
}

fn server_authority(uri: &JavaUri) -> ServerAuthority {
    let Some(authority) = uri.get_raw_authority() else {
        return ServerAuthority {
            user_info: None,
            host: None,
            port: -1,
        };
    };
    let (user_info, host_port) = match authority.rfind('@') {
        Some(i) => (Some(authority[..i].to_string()), &authority[i + 1..]),
        None => (None, authority),
    };
    let (host, port_str) = if host_port.starts_with('[') {
        match host_port.find(']') {
            Some(end) => (&host_port[..=end], host_port[end + 1..].strip_prefix(':')),
            None => (host_port, None),
        }
    } else {
        match host_port.rfind(':') {
            Some(i) => (&host_port[..i], Some(&host_port[i + 1..])),
            None => (host_port, None),
        }
    };
    let port = match port_str {
        None | Some("") => -1,
        Some(p) if p.bytes().all(|b| b.is_ascii_digit()) => p.parse::<i32>().unwrap_or(-1),
        // Not a server-based authority: Java leaves host and port undefined.
        Some(_) => {
            return ServerAuthority {
                user_info: None,
                host: None,
                port: -1,
            };
        }
    };
    ServerAuthority {
        user_info,
        host: if host.is_empty() {
            None
        } else {
            Some(host.to_string())
        },
        port,
    }
}

/// `URI.getHost()`.
pub fn get_host(uri: &JavaUri) -> Option<String> {
    server_authority(uri).host
}

// Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpUtils.java:61 HttpUtils.getLocation
fn get_location(
    new_location: Option<String>,
    connection_url: &JavaUri,
) -> Result<JavaUri, IoException> {
    let Some(new_location) = new_location else {
        return Err(IoException::new("Remote redirect missing Location."));
    };
    // `URI.create` throws IllegalArgumentException, which HttpConnector.connect turns into an
    // UnrecoverableHttpException carrying the message.
    let preferred = JavaUri::new(&new_location).map_err(|e| {
        IoException::of(IoExceptionKind::UnrecoverableHttpException, e.get_message())
    })?;
    let result = merge_urls(&preferred, connection_url)?;
    if !is_http(&result) {
        return Err(IoException::new(format!("Bad Location: {new_location}")));
    }
    Ok(result)
}

// Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpUtils.java:73 HttpUtils.mergeUrls
fn merge_urls(preferred: &JavaUri, original: &JavaUri) -> Result<JavaUri, IoException> {
    let p = server_authority(preferred);
    let o = server_authority(original);
    // Try to short cut to preferred to preserve the original presentation of the
    // quoting (as a call to the structured URI constructor puts quoting into a canonical form).
    // This is necessary as some sites rely on the precise presentation for the authentication
    // scheme of their redirect URLs.
    if p.host.is_some()
        && preferred.get_scheme().is_some()
        && (preferred.get_raw_fragment().is_some() || original.get_raw_fragment().is_none())
        // Forward user info to the same origin.
        && (p.user_info.is_some()
            || o.user_info.is_none()
            || !(p.host == o.host && p.port == o.port))
    {
        // In this case we obviously do not inherit anything from the original URL, as all
        // inheritable fields are either set explicitly or not present in the original either.
        // Therefore, it is safe to short cut.
        return Ok(preferred.clone());
    }

    // If the Location value provided in a 3xx (Redirection) response does not have a fragment
    // component, a user agent MUST process the redirection as if the value inherits the fragment
    // component of the URI reference used to generate the request target (i.e., the redirection
    // inherits the original reference's fragment, if any). Quoth RFC7231 § 7.1.2
    let protocol = preferred.get_scheme().or(original.get_scheme());
    let mut user_info = p.user_info.clone();
    let host;
    let port;
    match &p.host {
        None => {
            host = o.host.clone();
            port = o.port;
            user_info = o.user_info.clone();
        }
        Some(h) => {
            host = Some(h.clone());
            port = p.port;
            if user_info.is_none() && Some(h) == o.host.as_ref() && port == o.port {
                user_info = o.user_info.clone();
            }
        }
    }
    let path = preferred.get_path();
    let query = preferred.get_raw_query().map(percent_decode_str);
    let fragment = preferred
        .get_raw_fragment()
        .or(original.get_raw_fragment())
        .map(percent_decode_str);
    let could_not_merge =
        || IoException::new(format!("Could not merge {preferred} into {original}"));
    // Java: java.net.URI.<init>(String, String, String, int, String, String, String)
    let mut s = String::new();
    if let Some(scheme) = protocol {
        s.push_str(scheme);
        s.push(':');
    }
    if let Some(h) = &host {
        s.push_str("//");
        if let Some(ui) = &user_info {
            s.push_str(&quote(ui, QuoteSet::UserInfo));
            s.push('@');
        }
        s.push_str(h);
        if port != -1 {
            s.push(':');
            s.push_str(&port.to_string());
        }
    }
    if let Some(path) = &path {
        if host.is_some() && !path.is_empty() && !path.starts_with('/') {
            // URISyntaxException: Relative path in absolute URI.
            return Err(could_not_merge());
        }
        s.push_str(&quote(path, QuoteSet::Path));
    }
    if let Some(q) = &query {
        s.push('?');
        s.push_str(&quote(q, QuoteSet::Uric));
    }
    if let Some(f) = &fragment {
        s.push('#');
        s.push_str(&quote(f, QuoteSet::Uric));
    }
    JavaUri::new(&s).map_err(|_| could_not_merge())
}

#[derive(Clone, Copy)]
enum QuoteSet {
    UserInfo,
    Path,
    Uric,
}

/// The quoting of `java.net.URI`'s multi-argument constructors: characters that are not legal in
/// the component (and always `%`) become `%XX` escapes of their UTF-8 bytes; other non-ASCII
/// characters are kept.
fn quote(s: &str, set: QuoteSet) -> String {
    fn unreserved(c: char) -> bool {
        c.is_ascii_alphanumeric() || "_-!.~'()*".contains(c)
    }
    let legal = |c: char| match set {
        QuoteSet::UserInfo => unreserved(c) || ";:&=+$,".contains(c),
        QuoteSet::Path => unreserved(c) || ";:@&=+$,/".contains(c),
        QuoteSet::Uric => unreserved(c) || ";/?:@&=+$,[]".contains(c),
    };
    let mut out = String::new();
    for c in s.chars() {
        if (!c.is_ascii() && !c.is_control() && !c.is_whitespace()) || (c.is_ascii() && legal(c)) {
            out.push(c);
        } else {
            let mut buf = [0u8; 4];
            for b in c.encode_utf8(&mut buf).bytes() {
                out.push_str(&format!("%{b:02X}"));
            }
        }
    }
    out
}

fn percent_decode_str(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(h), Some(l)) = (
                (bytes[i + 1] as char).to_digit(16),
                (bytes[i + 2] as char).to_digit(16),
            )
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ---------------------------------------------------------------------------------------------
// ProxyHelper
// ---------------------------------------------------------------------------------------------

/// Checks the client environment for a proxy that applies to `requested_url`. Proxies are not
/// ported: a configured one fails loudly.
// PORT: the `http.nonProxyHosts` / `http(s).proxyHost` JVM system properties are never set for
// Blaze, so only the environment variables are consulted.
// Java: com/google/devtools/build/lib/bazel/repository/downloader/ProxyHelper.java:74 ProxyHelper.createProxyIfNeeded
fn check_no_proxy_needed(env: &IndexMap<String, String>, requested_url: &JavaUri) {
    // Check no_proxy/NO_PROXY environment variables
    let mut no_proxy_url = env.get("no_proxy").map(String::as_str);
    if no_proxy_url.is_none_or(str::is_empty) {
        no_proxy_url = env.get("NO_PROXY").map(String::as_str);
    }
    if let Some(no_proxy_url) = no_proxy_url.filter(|s| !s.is_empty()) {
        let requested_host = get_host(requested_url).unwrap_or_default();
        for entry in java_split_comma(no_proxy_url) {
            if entry.starts_with('.') {
                // This entry applies to sub-domains only.
                if requested_host.ends_with(entry.as_str()) {
                    return;
                }
            } else if requested_host == entry || requested_host.ends_with(&format!(".{entry}")) {
                // This entry applies to the literal hostname and sub-domains.
                return;
            }
        }
    }
    let proxy_address = if is_protocol(requested_url, "https") {
        env.get("https_proxy").or_else(|| env.get("HTTPS_PROXY"))
    } else if is_protocol(requested_url, "http") {
        env.get("http_proxy").or_else(|| env.get("HTTP_PROXY"))
    } else {
        None
    };
    if let Some(address) = proxy_address.filter(|a| !a.is_empty()) {
        blaze_util::unported!(
            "ProxyHelper.createProxyInfo",
            "proxy {address} for {requested_url}"
        );
    }
}

/// `noProxyUrl.split("\\s*,\\s*")` (trailing empty strings removed).
fn java_split_comma(s: &str) -> Vec<String> {
    let mut parts: Vec<String> = s
        .split(',')
        .enumerate()
        .map(|(i, p)| {
            // The separator swallows surrounding whitespace, but not the first part's leading
            // or the last part's trailing whitespace.
            let p = if i == 0 { p } else { p.trim_start() };
            p.to_string()
        })
        .collect();
    let n = parts.len();
    for (i, p) in parts.iter_mut().enumerate() {
        if i + 1 < n {
            *p = p.trim_end().to_string();
        }
    }
    while parts.last().is_some_and(String::is_empty) {
        parts.pop();
    }
    parts
}

// ---------------------------------------------------------------------------------------------
// URLConnection
// ---------------------------------------------------------------------------------------------

/// An open connection with a received response (Java `URLConnection` / `HttpURLConnection`).
pub(crate) struct UrlConnection {
    url: JavaUri,
    response_code: i32,
    response_message: Option<String>,
    headers: Vec<(String, String)>,
    body: Option<Result<Box<dyn Read + Send>, IoException>>,
}

impl UrlConnection {
    pub(crate) fn get_url(&self) -> &JavaUri {
        &self.url
    }

    pub(crate) fn get_response_code(&self) -> i32 {
        self.response_code
    }

    /// `getHeaderField(name)`: case-insensitive; the last value wins.
    pub(crate) fn get_header_field(&self, name: &str) -> Option<String> {
        self.headers
            .iter()
            .rev()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.clone())
    }

    /// `getInputStream()` / `getErrorStream()` (only once).
    fn get_input_stream(&mut self) -> Result<Box<dyn Read + Send>, IoException> {
        match self.body.take() {
            Some(b) => b,
            None => Err(IoException::new("Stream closed")),
        }
    }

    /// `HttpURLConnection.disconnect()`.
    fn disconnect(&mut self) {
        self.body = None;
    }

    // Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpConnector.java:309 HttpConnector.describeHttpResponse
    fn describe_http_response(&self) -> String {
        format!(
            "GET returned {} {}",
            self.response_code,
            self.response_message.as_deref().unwrap_or("")
        )
    }
}

/// `url.toURL().openConnection()` for a `file:` URL (`FileURLConnection`).
fn open_file_connection(url: &JavaUri) -> UrlConnection {
    let path = url.get_path().unwrap_or_default();
    let mut headers = Vec::new();
    let body: Result<Box<dyn Read + Send>, IoException> = match std::fs::metadata(&path) {
        Ok(m) if m.is_dir() => {
            blaze_util::unported!("FileURLConnection.connect", "directory listing of {path}")
        }
        Ok(m) => {
            headers.push(("content-length".to_string(), m.len().to_string()));
            match std::fs::File::open(&path) {
                Ok(f) => Ok(Box::new(f) as Box<dyn Read + Send>),
                Err(e) => Err(IoException::of(
                    IoExceptionKind::FileNotFoundException,
                    format!("{path} ({})", java_io_message(&e)),
                )),
            }
        }
        Err(e) => Err(IoException::of(
            IoExceptionKind::FileNotFoundException,
            format!("{path} ({})", java_io_message(&e)),
        )),
    };
    UrlConnection {
        url: url.clone(),
        response_code: -1,
        response_message: None,
        headers,
        body: Some(body),
    }
}

fn map_transport_error(url: &JavaUri, t: &ureq::Transport) -> IoException {
    let message = t.to_string();
    let io_kind = std::error::Error::source(t)
        .and_then(|s| s.downcast_ref::<std::io::Error>())
        .map(|e| e.kind());
    match t.kind() {
        ureq::ErrorKind::Dns => IoException::of(
            IoExceptionKind::UnknownHostException,
            get_host(url).unwrap_or_default(),
        ),
        // `HttpURLConnection` rejects these with IllegalArgumentException.
        ureq::ErrorKind::InvalidUrl | ureq::ErrorKind::UnknownScheme => {
            IoException::of(IoExceptionKind::IllegalArgumentException, message)
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
            _ if message.contains("tls")
                || message.contains("TLS")
                || message.contains("certificate") =>
            {
                IoException::of(IoExceptionKind::SSLException, message)
            }
            _ => IoException::new(message),
        },
    }
}

/// Maps an error while reading a response body.
fn map_read_error(e: &std::io::Error) -> IoException {
    match e.kind() {
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
            IoException::of(IoExceptionKind::SocketTimeoutException, "Read timed out")
        }
        std::io::ErrorKind::ConnectionReset => {
            IoException::of(IoExceptionKind::SocketException, "Connection reset")
        }
        std::io::ErrorKind::Interrupted => {
            IoException::without_message(IoExceptionKind::InterruptedIOException)
        }
        _ => IoException::new(e.to_string()),
    }
}

/// Sends one GET without following redirects (`HttpURLConnection.connect` +
/// `getResponseCode`).
fn open_http_connection(
    url: &JavaUri,
    headers: &[(String, String)],
    connect_timeout_ms: i32,
    read_timeout_ms: i32,
) -> Result<UrlConnection, IoException> {
    let agent = ureq::AgentBuilder::new()
        .redirects(0)
        .timeout_connect(Duration::from_millis(connect_timeout_ms.max(1) as u64))
        .timeout_read(Duration::from_millis(read_timeout_ms.max(1) as u64))
        .build();
    let mut request = agent.request("GET", &url.to_string());
    // PORT: HttpURLConnection sends repeated request properties as separate header lines; ureq
    // keeps one line per name, so repeated values are joined with ", " (RFC 9110 §5.3).
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
    let response_code = response.status() as i32;
    let response_message = Some(response.status_text().to_string());
    let mut resp_headers = Vec::new();
    for name in response.headers_names() {
        for v in response.all(&name) {
            resp_headers.push((name.clone(), v.to_string()));
        }
    }
    let reader: Box<dyn Read + Send> = Box::new(response.into_reader());
    Ok(UrlConnection {
        url: url.clone(),
        response_code,
        response_message,
        headers: resp_headers,
        body: Some(Ok(reader)),
    })
}

// ---------------------------------------------------------------------------------------------
// HttpConnector
// ---------------------------------------------------------------------------------------------

const MAX_ATTEMPTS: i32 = 8;
const MAX_REDIRECTS: i32 = 40;
const MIN_RETRY_DELAY_MS: i32 = 100;
const MIN_CONNECT_TIMEOUT_MS: i32 = 1000;
const MAX_CONNECT_TIMEOUT_MS: i32 = 10000;
const READ_TIMEOUT_MS: i32 = 20000;
const COMPRESSED_EXTENSIONS: [&str; 7] = ["bz2", "gz", "jar", "tgz", "war", "xz", "zip"];

/// `"bazel/" + BlazeVersionInfo.instance().getVersion()`.
// Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpConnector.java:64 HttpConnector.USER_AGENT_VALUE
const USER_AGENT_VALUE: &str = "bazel/9.3.2";

/// Headers to send for a URL (Java `Function<URI, ImmutableMap<String, List<String>>>`).
type HeaderFunction<'a> = dyn Fn(&JavaUri) -> Headers + 'a;

/// `Math.random()`.
fn java_random() -> f64 {
    (uuid::Uuid::new_v4().as_u128() >> 75) as f64 / (1u64 << 53) as f64
}

/// `String.format(locale, "%,d", n)` with the default (English) grouping.
fn format_grouped(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

/// Class for establishing connections to HTTP servers for downloading files. Port of
/// `HttpConnector`.
// Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpConnector.java:54 HttpConnector
struct HttpConnector<'a> {
    event_handler: &'a Arc<dyn ExtendedEventHandler>,
    client_env: &'a IndexMap<String, String>,
    timeout_scaling: f32,
    max_attempts: i32,
    max_retry_timeout: Duration,
}

impl<'a> HttpConnector<'a> {
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpConnector.java:75 HttpConnector.<init>
    fn new(
        event_handler: &'a Arc<dyn ExtendedEventHandler>,
        client_env: &'a IndexMap<String, String>,
        timeout_scaling: f32,
        max_attempts: i32,
        max_retry_timeout: Duration,
    ) -> Self {
        HttpConnector {
            event_handler,
            client_env,
            timeout_scaling,
            max_attempts: if max_attempts > 0 {
                max_attempts
            } else {
                MAX_ATTEMPTS
            },
            max_retry_timeout,
        }
    }

    // Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpConnector.java:106 HttpConnector.scale
    fn scale(&self, unscaled: i32) -> i32 {
        // Math.round(float) rounds half up.
        (unscaled as f32 * self.timeout_scaling + 0.5).floor() as i32
    }

    fn progress(&self, message: String) {
        self.event_handler.handle(&Event::progress(message));
    }

    // Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpConnector.java:110 HttpConnector.connect
    fn connect(
        &self,
        original_url: &JavaUri,
        request_headers: &HeaderFunction<'_>,
    ) -> Result<UrlConnection, IoException> {
        let mut original_url = original_url.clone();
        let mut url = original_url.clone();
        if is_protocol(&url, "file") {
            return Ok(open_file_connection(&url));
        }
        let mut suppressions: Vec<IoException> = Vec::new();
        let mut retries = 0;
        let mut redirects = 0;
        let mut connect_timeout = self.scale(MIN_CONNECT_TIMEOUT_MS);
        loop {
            let mut connection: Option<UrlConnection> = None;
            let attempt: Result<bool, IoException> = (|| -> Result<bool, IoException> {
                check_no_proxy_needed(self.client_env, &url);
                let is_already_compressed = COMPRESSED_EXTENSIONS
                    .contains(&get_extension(&url.get_path().unwrap_or_default()).as_str())
                    || COMPRESSED_EXTENSIONS.contains(
                        &get_extension(&original_url.get_path().unwrap_or_default()).as_str(),
                    );
                let mut header_lines: Vec<(String, String)> = Vec::new();
                for (key, values) in request_headers(&url) {
                    if is_already_compressed && key.eq_ignore_ascii_case("Accept-Encoding") {
                        // We're not going to ask for compression if we're downloading a file that
                        // already appears to be compressed.
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
                    header_lines.push(("User-Agent".to_string(), USER_AGENT_VALUE.to_string()));
                }
                // The read timeout is always large because it stays in effect after this method.
                let opened = open_http_connection(
                    &url,
                    &header_lines,
                    connect_timeout,
                    self.scale(READ_TIMEOUT_MS),
                );
                let conn = match opened {
                    Ok(c) => connection.insert(c),
                    Err(e) if e.kind == IoExceptionKind::SSLException => {
                        // Check if the exception is due to a permanent error, such as a
                        // certificate validation issue. These errors are unlikely to be resolved
                        // by retrying.
                        let m = e.get_message();
                        if m.contains("certificate") || m.contains("CertPathValidatorException") {
                            let message = format!("TLS error: {m}");
                            self.progress(message.clone());
                            let mut http_exception = IoException::of(
                                IoExceptionKind::UnrecoverableHttpException,
                                message,
                            );
                            http_exception.add_suppressed(e);
                            return Err(http_exception);
                        }
                        // Otherwise, treat it as a potentially transient network error and let
                        // it fall through to the standard IOException handler for retries.
                        return Err(e);
                    }
                    Err(e) if e.kind == IoExceptionKind::UnknownHostException => {
                        let message = format!("Unknown host: {}", e.get_message());
                        self.progress(message.clone());
                        let mut http_exception =
                            IoException::of(IoExceptionKind::UnrecoverableHttpException, message);
                        http_exception.add_suppressed(e);
                        return Err(http_exception);
                    }
                    Err(e) if e.kind == IoExceptionKind::IllegalArgumentException => {
                        // This will happen if the user does something like specify a port
                        // greater than 2^16-1.
                        return Err(IoException {
                            kind: IoExceptionKind::UnrecoverableHttpException,
                            message: e.message,
                            suppressed: Vec::new(),
                        });
                    }
                    Err(e) => return Err(e),
                };
                let code = conn.get_response_code();
                // 206 means partial content and only happens if caller specified Range. See
                // RFC7233 § 4.1.
                if code == 200 || code == 206 {
                    Ok(true)
                } else if code == 301 || code == 302 || code == 303 || code == 307 {
                    read_all_bytes_and_close(conn)?;
                    redirects += 1;
                    if redirects == MAX_REDIRECTS {
                        self.progress(format!("Redirect loop detected in {original_url}"));
                        return Err(IoException::of(
                            IoExceptionKind::UnrecoverableHttpException,
                            "Redirect loop detected",
                        ));
                    }
                    url = get_location(conn.get_header_field("Location"), conn.get_url())?;
                    if code == 301 {
                        original_url = url.clone();
                    }
                    Ok(false)
                } else if code == 403 {
                    // jart@ has noticed BitBucket + Amazon AWS downloads frequently flake with
                    // this code.
                    Err(IoException::new(conn.describe_http_response()))
                } else if code == 408 {
                    // The 408 (Request Timeout) status code indicates that the server did not
                    // receive a complete request message within the time that it was prepared to
                    // wait. If the client has an outstanding request in transit, the client MAY
                    // repeat that request on a new connection. Quoth RFC7231 § 6.5.7
                    Err(IoException::new(conn.describe_http_response()))
                } else if code == 429 {
                    // The 429 (Too Many Requests) status code could result from Bazel
                    // temporarily overloading the server and is typically resolved by retrying.
                    Err(IoException::new(conn.describe_http_response()))
                } else if code < 500 // 4xx means client seems to have erred quoth RFC7231 § 6.5
                    || code == 501 // Server doesn't support function quoth RFC7231 § 6.6.2
                    || code == 505
                // Server refuses to support version quoth RFC7231 § 6.6.6
                {
                    // This is a permanent error so we're not going to retry.
                    read_all_bytes_and_close(conn)?;
                    if code == 404 || code == 410 {
                        // For Not Found, we throw a separate unrecoverable exception so that
                        // callers can distinguish between the resource being not found and the
                        // server being unavailable.
                        return Err(IoException::of(
                            IoExceptionKind::FileNotFoundException,
                            conn.describe_http_response(),
                        ));
                    }
                    Err(IoException::of(
                        IoExceptionKind::UnrecoverableHttpException,
                        conn.describe_http_response(),
                    ))
                } else {
                    // However we will retry on some 5xx errors, particularly 500, 502 and 503.
                    Err(IoException::new(conn.describe_http_response()))
                }
            })();
            let mut e = match attempt {
                Ok(true) => return Ok(connection.take().unwrap()),
                Ok(false) => continue,
                Err(e)
                    if e.kind == IoExceptionKind::UnrecoverableHttpException
                        || e.kind == IoExceptionKind::FileNotFoundException =>
                {
                    return Err(e);
                }
                Err(e) if e.kind == IoExceptionKind::IllegalArgumentException => {
                    return Err(IoException {
                        kind: IoExceptionKind::UnrecoverableHttpException,
                        message: e.message,
                        suppressed: Vec::new(),
                    });
                }
                Err(e) => e,
            };
            if let Some(c) = connection.as_mut() {
                // If we got here, it means we might not have consumed the entire payload of the
                // response, if any. So we're going to force this socket to disconnect and not be
                // reused.
                c.disconnect();
            }
            // We don't respect the Retry-After header (RFC7231 § 7.1.3) because it's rarely used
            // and tends to be too conservative when it is. We're already being good citizens by
            // using exponential backoff with jitter. Furthermore RFC law didn't use the magic word
            // "MUST".
            let mut raw_timeout = (MIN_RETRY_DELAY_MS as f64) * 2f64.powi(retries);
            if !self.max_retry_timeout.is_zero() {
                raw_timeout = raw_timeout.min(self.max_retry_timeout.as_millis() as f64);
            }
            let mut timeout = ((0.75 + java_random() / 2.0) * raw_timeout) as i32;
            if e.kind == IoExceptionKind::SocketTimeoutException {
                self.progress(format!("Timeout connecting to {url}"));
                connect_timeout = (connect_timeout * 2).min(self.scale(MAX_CONNECT_TIMEOUT_MS));
                // If we got connect timeout, we're already doing exponential backoff, so no point
                // in sleeping too.
                timeout = 1;
            } else if e.kind == IoExceptionKind::InterruptedIOException {
                // Please note that SocketTimeoutException is a subtype of InterruptedIOException.
                return Err(e);
            }
            retries += 1;
            if retries == self.max_attempts {
                if e.kind == IoExceptionKind::SocketTimeoutException {
                    // SocketTimeoutExceptions are InterruptedIOExceptions; however they do not
                    // signify an external interruption, but simply a failed download due to some
                    // server timing out. So rethrow them as ordinary IOExceptions.
                    e = IoException {
                        kind: IoExceptionKind::IOException,
                        message: e.message.clone(),
                        suppressed: Vec::new(),
                    };
                } else {
                    self.progress(format!("Error connecting to {url}: {}", e.get_message()));
                }
                for suppressed in suppressions {
                    e.add_suppressed(suppressed);
                }
                return Err(e);
            }
            // Java 7 allows us to create a tree of all errors that led to the ultimate failure.
            suppressions.push(e);
            self.progress(format!(
                "Failed to connect to {url} trying again in {}ms",
                format_grouped(timeout as i64)
            ));
            url = original_url.clone();
            std::thread::sleep(Duration::from_millis(timeout.max(0) as u64));
        }
    }
}

/// Exhausts all bytes in an HTTP response to make it easier to reuse sockets.
// PORT: a body that ends before its Content-Length is a normal end of stream for
// HttpURLConnection; ureq reports it as UnexpectedEof, which is treated as end of stream here.
// Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpConnector.java:322 HttpConnector.readAllBytesAndClose
fn read_all_bytes_and_close(connection: &mut UrlConnection) -> Result<(), IoException> {
    if let Ok(mut stream) = connection.get_input_stream() {
        let mut buf = [0u8; 8192];
        loop {
            match stream.read(&mut buf) {
                Ok(0) => break,
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(map_read_error(&e)),
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// HttpStream
// ---------------------------------------------------------------------------------------------

const MAX_RESUMES: i32 = 3;
const GZIPPED_EXTENSIONS: [&str; 2] = ["gz", "tgz"];
const GZIP_CONTENT_ENCODING: [&str; 2] = ["gzip", "x-gzip"];

/// Establishes a connection again after a read failure (`RetryingInputStream.Reconnector`).
type Reconnector<'a> = dyn Fn(&IoException, &Headers) -> Result<UrlConnection, IoException> + 'a;

/// Reads one chunk from `stream`, mapping errors; `Ok(0)` is end of stream.
fn read_chunk(stream: &mut dyn Read, buf: &mut [u8]) -> Result<usize, IoException> {
    loop {
        match stream.read(buf) {
            Ok(n) => return Ok(n),
            // See read_all_bytes_and_close: a short body is an end of stream, which the content
            // length check below reports.
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(0),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(map_read_error(&e)),
        }
    }
}

/// Reads the whole payload of `connection` through the streams `HttpStream.Factory.create`
/// stacks (`RetryingInputStream`, `CheckContentLengthInputStream`, `GZIPInputStream`,
/// `HashInputStream`) and returns it.
// Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpStream.java:71 HttpStream.Factory.create
fn read_http_stream(
    mut connection: UrlConnection,
    original_url: &JavaUri,
    checksum: Option<&Checksum>,
    reconnector: &Reconnector<'_>,
) -> Result<Vec<u8>, IoException> {
    let mut stream = connection.get_input_stream()?;
    let connection_url = connection.get_url().clone();

    // If server supports range requests, we can retry on read errors. See RFC7233 § 2.3.
    let retrying = connection
        .get_header_field("Accept-Ranges")
        .unwrap_or_default()
        .split(',')
        .any(|s| s.trim() == "bytes");

    // Long.parseUnsignedLong; a malformed value is ignored.
    let total_bytes: Option<u64> = connection.get_header_field("Content-Length").and_then(|s| {
        if s.starts_with('+') || s.starts_with('-') {
            None
        } else {
            s.parse::<u64>().ok()
        }
    });

    // RetryingInputStream state.
    let mut toto: u64 = 0;
    let mut resumes = 0;
    let mut suppressed: Vec<IoException> = Vec::new();

    let mut raw = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        match read_chunk(stream.as_mut(), &mut buf) {
            Ok(0) => break,
            Ok(n) => {
                raw.extend_from_slice(&buf[..n]);
                toto += n as u64;
            }
            Err(cause) if retrying => {
                // Java: com/google/devtools/build/lib/bazel/repository/downloader/RetryingInputStream.java:97 RetryingInputStream.tryAgainIfPossible
                if cause.kind == IoExceptionKind::InterruptedIOException {
                    return Err(cause);
                }
                let propagate = |mut error: IoException, suppressed: &[IoException]| {
                    for e in suppressed {
                        error.add_suppressed(e.clone());
                    }
                    error
                };
                if resumes >= MAX_RESUMES {
                    return Err(propagate(cause, &suppressed));
                }
                resumes += 1;
                suppressed.push(cause.clone());
                // Java: com/google/devtools/build/lib/bazel/repository/downloader/RetryingInputStream.java:114 RetryingInputStream.reconnectWhereWeLeftOff
                let amount_read = toto;
                let reconnected = if amount_read == 0 {
                    reconnector(&cause, &Headers::new())
                } else {
                    let mut extra = Headers::new();
                    extra.insert("Range".to_string(), vec![format!("bytes={amount_read}-")]);
                    reconnector(&cause, &extra).and_then(|c| {
                        if !c
                            .get_header_field("Content-Range")
                            .unwrap_or_default()
                            .starts_with(&format!("bytes {amount_read}-"))
                        {
                            return Err(IoException::new(format!(
                                "Tried to reconnect at offset {} but server didn't support it",
                                format_grouped(amount_read as i64)
                            )));
                        }
                        Ok(c)
                    })
                };
                match reconnected {
                    Ok(mut c) => match c.get_input_stream() {
                        Ok(s) => stream = s,
                        Err(e) if e.kind == IoExceptionKind::InterruptedIOException => {
                            return Err(e);
                        }
                        Err(e) => return Err(propagate(e, &suppressed)),
                    },
                    Err(e) if e.kind == IoExceptionKind::InterruptedIOException => return Err(e),
                    Err(e) => return Err(propagate(e, &suppressed)),
                }
            }
            Err(e) => return Err(e),
        }
    }

    // Java: com/google/devtools/build/lib/bazel/repository/downloader/CheckContentLengthInputStream.java:69 CheckContentLengthInputStream.checkContentLength
    if let Some(expected_size) = total_bytes
        && raw.len() as u64 != expected_size
    {
        return Err(IoException::of(
            IoExceptionKind::ContentLengthMismatchException,
            format!("Bytes read {} but wanted {}", raw.len(), expected_size),
        ));
    }

    // Determine if we need to transparently gunzip. See RFC2616 § 3.5 and § 14.11. Please note
    // that some web servers will send Content-Encoding: gzip even when we didn't request it if
    // the file is a .gz file. If the file that we are downloading is already compressed we should
    // not decompress it to preserve the desired file format.
    // PORT: the `type` argument is always empty on the downloadAndRead path.
    let content_encoding = connection
        .get_header_field("Content-Encoding")
        .unwrap_or_default();
    let payload = if GZIP_CONTENT_ENCODING.contains(&content_encoding.as_str())
        && !GZIPPED_EXTENSIONS
            .contains(&get_extension(&connection_url.get_path().unwrap_or_default()).as_str())
        && !GZIPPED_EXTENSIONS
            .contains(&get_extension(&original_url.get_path().unwrap_or_default()).as_str())
    {
        gunzip(&raw)?
    } else {
        raw
    };

    if let Some(checksum) = checksum {
        // Java: com/google/devtools/build/lib/bazel/repository/downloader/HashInputStream.java:82 HashInputStream.check
        blaze_bzlmod_model::registry::verify_checksum(&payload, Some(checksum)).map_err(|e| {
            IoException::of(
                IoExceptionKind::UnrecoverableHttpException,
                e.get_message().to_string(),
            )
        })?;
    }
    Ok(payload)
}

/// `new GZIPInputStream(stream)` read to the end (concatenated members included).
// PORT: GZIPInputStream reads the header in its constructor: an empty stream is an EOFException
// and a bad magic number is "Not in GZIP format"; other format errors carry flate2's message.
fn gunzip(raw: &[u8]) -> Result<Vec<u8>, IoException> {
    if raw.len() < 2 {
        return Err(IoException::without_message(IoExceptionKind::EOFException));
    }
    if raw[0] != 0x1f || raw[1] != 0x8b {
        return Err(IoException::of(
            IoExceptionKind::ZipException,
            "Not in GZIP format",
        ));
    }
    let mut out = Vec::new();
    flate2::read::MultiGzDecoder::new(raw)
        .read_to_end(&mut out)
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::UnexpectedEof => IoException::of(
                IoExceptionKind::EOFException,
                "Unexpected end of ZLIB input stream",
            ),
            _ => IoException::of(IoExceptionKind::ZipException, e.to_string()),
        })?;
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// HttpConnectorMultiplexer
// ---------------------------------------------------------------------------------------------

/// `REQUEST_HEADERS`: `Accept-Encoding: gzip`, `User-Agent: Bazel/<release name>`.
// Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpConnectorMultiplexer.java:52 HttpConnectorMultiplexer.REQUEST_HEADERS
fn request_headers() -> Headers {
    let mut h = Headers::new();
    h.insert("Accept-Encoding".to_string(), vec!["gzip".to_string()]);
    h.insert(
        "User-Agent".to_string(),
        vec![format!("Bazel/{RELEASE_NAME}")],
    );
    h
}

/// Establishes the connection and reads the payload (`HttpConnectorMultiplexer.connect`, then
/// `HttpStream` read to the end).
// PORT: credentials are always empty (no netrc, no credential helpers), so the header function
// adds nothing to the base headers.
// Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpConnectorMultiplexer.java:96 HttpConnectorMultiplexer.connect
fn connect_and_read(
    connector: &HttpConnector<'_>,
    url: &JavaUri,
    checksum: Option<&Checksum>,
    headers: &Headers,
) -> Result<Vec<u8>, IoException> {
    blaze_util::check_argument!(is_url_supported_by_downloader(url));
    let mut base_headers = Headers::new();
    for (k, v) in headers {
        base_headers.insert(k.clone(), v.clone());
    }
    // REQUEST_HEADERS should not be overridable by user provided headers
    for (k, v) in request_headers() {
        base_headers.insert(k, v);
    }
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpConnectorMultiplexer.java:134 HttpConnectorMultiplexer.getHeaderFunction
    let header_function = |_url: &JavaUri| -> Headers { base_headers.clone() };
    let connection = connector.connect(url, &header_function)?;
    let connection_url = connection.get_url().clone();
    let reconnector = |cause: &IoException, extra_headers: &Headers| {
        connector.progress(format!(
            "Lost connection for {url} due to {}",
            cause.to_java_string()
        ));
        connector.connect(&connection_url, &|new_url: &JavaUri| {
            let mut h = header_function(new_url);
            for (k, v) in extra_headers {
                blaze_util::check_argument!(
                    !h.contains_key(k),
                    "Multiple entries with same key: {k}"
                );
                h.insert(k.clone(), v.clone());
            }
            h
        })
    };
    read_http_stream(connection, url, checksum, &reconnector)
}

// ---------------------------------------------------------------------------------------------
// HttpDownloader
// ---------------------------------------------------------------------------------------------

/// `java.util.concurrent.Semaphore` (fair).
struct Semaphore {
    permits: Mutex<i32>,
    cv: Condvar,
}

impl Semaphore {
    fn acquire(&self) {
        let mut p = self.permits.lock().unwrap();
        while *p <= 0 {
            p = self.cv.wait(p).unwrap();
        }
        *p -= 1;
    }

    fn release(&self) {
        *self.permits.lock().unwrap() += 1;
        self.cv.notify_one();
    }
}

/// HTTP implementation of the downloader, reduced to `downloadAndRead`. Port of
/// `HttpDownloader`.
// Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpDownloader.java:55 HttpDownloader
pub struct LocalHttpDownloader {
    semaphore: Semaphore,
    timeout_scaling: f32,
    max_attempts: i32,
    max_retry_timeout: Duration,
}

impl LocalHttpDownloader {
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpDownloader.java:65 HttpDownloader.<init>
    pub fn new(
        max_attempts: i32,
        max_retry_timeout: Duration,
        max_parallel_downloads: i32,
        timeout_scaling: f32,
    ) -> Self {
        LocalHttpDownloader {
            semaphore: Semaphore {
                permits: Mutex::new(max_parallel_downloads),
                cv: Condvar::new(),
            },
            timeout_scaling,
            max_attempts,
            max_retry_timeout,
        }
    }

    /// Downloads the contents of the first of the given URLs that succeeds. If all of them fail,
    /// the last failure is returned with the earlier failures attached as suppressed exceptions.
    // PORT: credentials are StaticCredentials.EMPTY (see LocalDownloadManagerFetcher).
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpDownloader.java:122 HttpDownloader.downloadAndRead
    pub fn download_and_read(
        &self,
        urls: &[JavaUri],
        checksum: Option<&Checksum>,
        event_handler: &Arc<dyn ExtendedEventHandler>,
        client_env: &IndexMap<String, String>,
    ) -> Result<Vec<u8>, IoException> {
        blaze_util::check_argument!(
            !urls.is_empty(),
            "Cannot download from an empty list of URLs"
        );
        // Buffer the (small) registry file in memory and return the bytes. Unlike download(),
        // this posts no progress events and preserves the type of the failing exception for the
        // caller.
        self.download_from_urls(urls, &Headers::new(), checksum, event_handler, client_env)
    }

    // PORT: reportProgressEvents is false on this path, so neither FetchEvent posts nor the
    // per-URL warnings are emitted.
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpDownloader.java:166 HttpDownloader.downloadFromUrls
    fn download_from_urls(
        &self,
        urls: &[JavaUri],
        headers: &Headers,
        checksum: Option<&Checksum>,
        event_handler: &Arc<dyn ExtendedEventHandler>,
        client_env: &IndexMap<String, String>,
    ) -> Result<Vec<u8>, IoException> {
        // Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpDownloader.java:279 HttpDownloader.setUpConnectorMultiplexer
        let connector = HttpConnector::new(
            event_handler,
            client_env,
            self.timeout_scaling,
            self.max_attempts,
            self.max_retry_timeout,
        );

        // Iterate over the URLs, attempting each in turn and falling back to the next one if the
        // previous one failed, until one succeeds.
        let mut io_exceptions: Vec<IoException> = Vec::new();
        for url in urls {
            self.semaphore.acquire();
            let result = connect_and_read(&connector, url, checksum, headers);
            self.semaphore.release();
            match result {
                Ok(content) => return Ok(content),
                Err(e) if e.kind == IoExceptionKind::SocketTimeoutException => {
                    // SocketTimeoutExceptions are InterruptedIOExceptions; however they do not
                    // signify an external interruption, but simply a failed download due to some
                    // server timing out. So treat them as ordinary IOExceptions and fall back to
                    // the next URL.
                    io_exceptions.push(IoException::wrapping(&e));
                }
                Err(e) if e.kind == IoExceptionKind::InterruptedIOException => return Err(e),
                Err(e) => io_exceptions.push(e),
            }
        }
        Err(last_download_exception(io_exceptions))
    }
}

/// The last failure, with the earlier failures attached as suppressed exceptions.
// Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpDownloader.java:271 HttpDownloader.lastDownloadException
fn last_download_exception(mut io_exceptions: Vec<IoException>) -> IoException {
    let mut last = io_exceptions.pop().expect("at least one failure");
    for e in io_exceptions {
        last.add_suppressed(e);
    }
    last
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uri(s: &str) -> JavaUri {
        JavaUri::new(s).unwrap()
    }

    fn location(connection_url: &str, location: Option<&str>) -> Result<String, IoException> {
        get_location(location.map(str::to_string), &uri(connection_url)).map(|u| u.to_string())
    }

    // Java: src/test/java/com/google/devtools/build/lib/bazel/repository/downloader/HttpUtilsTest.java:40 HttpUtilsTest.getExtension_twoExtensions_returnsLast
    #[test]
    fn get_extension_two_extensions_returns_last() {
        assert_eq!(get_extension("doodle.tar.gz"), "gz");
    }

    // Java: src/test/java/com/google/devtools/build/lib/bazel/repository/downloader/HttpUtilsTest.java:45 HttpUtilsTest.getExtension_isUppercase_returnsLowered
    #[test]
    fn get_extension_is_uppercase_returns_lowered() {
        assert_eq!(get_extension("DOODLE.TXT"), "txt");
    }

    // Java: src/test/java/com/google/devtools/build/lib/bazel/repository/downloader/HttpUtilsTest.java:50 HttpUtilsTest.getLocation_missingInRedirect_throwsIOException
    #[test]
    fn get_location_missing_in_redirect_throws_io_exception() {
        let e = location("http://lol.example", None).unwrap_err();
        assert_eq!(e.kind, IoExceptionKind::IOException);
        assert_eq!(e.get_message(), "Remote redirect missing Location.");
    }

    // Java: src/test/java/com/google/devtools/build/lib/bazel/repository/downloader/HttpUtilsTest.java:57 HttpUtilsTest.getLocation_absoluteInRedirect_returnsNewUrl
    #[test]
    fn get_location_absolute_in_redirect_returns_new_url() {
        assert_eq!(
            location("http://lol.example", Some("http://new.example/hi")).unwrap(),
            "http://new.example/hi"
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/bazel/repository/downloader/HttpUtilsTest.java:64 HttpUtilsTest.getLocation_redirectOnlyHasPath_mergesHostFromOriginalUrl
    #[test]
    fn get_location_redirect_only_has_path_merges_host_from_original_url() {
        assert_eq!(
            location("http://lol.example", Some("/hi")).unwrap(),
            "http://lol.example/hi"
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/bazel/repository/downloader/HttpUtilsTest.java:71 HttpUtilsTest.getLocation_onlyHasPathWithoutSlash_failsToMerge
    #[test]
    fn get_location_only_has_path_without_slash_fails_to_merge() {
        let e = location("http://lol.example", Some("omg")).unwrap_err();
        assert_eq!(e.kind, IoExceptionKind::IOException);
        assert!(e.get_message().contains("Could not merge"), "{e}");
    }

    // Java: src/test/java/com/google/devtools/build/lib/bazel/repository/downloader/HttpUtilsTest.java:80 HttpUtilsTest.getLocation_hasFragment_prefersNewFragment
    #[test]
    fn get_location_has_fragment_prefers_new_fragment() {
        assert_eq!(
            location("http://lol.example#a", Some("http://new.example/hi#b")).unwrap(),
            "http://new.example/hi#b"
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/bazel/repository/downloader/HttpUtilsTest.java:87 HttpUtilsTest.getLocation_hasNoFragmentButOriginalDoes_mergesOldFragment
    #[test]
    fn get_location_has_no_fragment_but_original_does_merges_old_fragment() {
        assert_eq!(
            location("http://lol.example#a", Some("http://new.example/hi")).unwrap(),
            "http://new.example/hi#a"
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/bazel/repository/downloader/HttpUtilsTest.java:94 HttpUtilsTest.getLocation_oldUrlHasPassRedirectingToSameDomain_mergesPassword
    #[test]
    fn get_location_old_url_has_pass_redirecting_to_same_domain_merges_password() {
        assert_eq!(
            location("http://a:b@lol.example", Some("http://lol.example/hi")).unwrap(),
            "http://a:b@lol.example/hi"
        );
        assert_eq!(
            location("http://a:b@lol.example", Some("/hi")).unwrap(),
            "http://a:b@lol.example/hi"
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/bazel/repository/downloader/HttpUtilsTest.java:106 HttpUtilsTest.getLocation_oldUrlHasPasswordRedirectingToNewServer_doesntMerge
    #[test]
    fn get_location_old_url_has_password_redirecting_to_new_server_doesnt_merge() {
        assert_eq!(
            location("http://a:b@lol.example", Some("http://new.example/hi")).unwrap(),
            "http://new.example/hi"
        );
        assert_eq!(
            location("http://a:b@lol.example", Some("http://lol.example:81/hi")).unwrap(),
            "http://lol.example:81/hi"
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/bazel/repository/downloader/HttpUtilsTest.java:116 HttpUtilsTest.getLocation_redirectToFtp_throwsIOException
    #[test]
    fn get_location_redirect_to_ftp_throws_io_exception() {
        let e = location("http://lol.example", Some("ftp://lol.example")).unwrap_err();
        assert_eq!(e.kind, IoExceptionKind::IOException);
        assert!(e.get_message().contains("Bad Location"), "{e}");
    }

    // Java: src/test/java/com/google/devtools/build/lib/bazel/repository/downloader/HttpUtilsTest.java:125 HttpUtilsTest.getLocation_redirectToHttps_works
    #[test]
    fn get_location_redirect_to_https_works() {
        assert_eq!(
            location("http://lol.example", Some("https://lol.example")).unwrap(),
            "https://lol.example"
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/bazel/repository/downloader/HttpUtilsTest.java:132 HttpUtilsTest.getLocation_preservesQuotingIfNotInheriting
    #[test]
    fn get_location_preserves_quoting_if_not_inheriting() {
        let redirect = "http://redirected.example.org/foo?\
                        response-content-disposition=attachment%3Bfilename%3D%22bar.tar.gz%22";
        assert_eq!(
            location("http://original.example.org", Some(redirect)).unwrap(),
            redirect
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/bazel/repository/downloader/HttpUtilsTest.java:142 HttpUtilsTest.getLocation_preservesQuotingWithUserIfNotInheriting
    #[test]
    fn get_location_preserves_quoting_with_user_if_not_inheriting() {
        let redirect = "http://redirected.example.org/foo?\
                        response-content-disposition=attachment%3Bfilename%3D%22bar.tar.gz%22";
        assert_eq!(
            location("http://a:b@original.example.org", Some(redirect)).unwrap(),
            redirect
        );
    }

    #[test]
    fn format_grouped_uses_commas() {
        assert_eq!(format_grouped(1), "1");
        assert_eq!(format_grouped(1234), "1,234");
        assert_eq!(format_grouped(1234567), "1,234,567");
    }

    #[test]
    fn java_split_comma_trims_around_separators() {
        assert_eq!(java_split_comma("a , b,c,,"), vec!["a", "b", "c"]);
        assert_eq!(java_split_comma(" a"), vec![" a"]);
    }
}
