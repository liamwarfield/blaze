//! The default [`HttpTransport`]: one HTTP/1.1 GET over a TCP or TLS (rustls, webpki roots)
//! connection, with the body framing of the JDK's `HttpURLConnection`.
//!
//! PORT: Java uses `java.net.HttpURLConnection` (`sun.net.www.protocol.http`), which is not part
//! of Bazel's sources. This module stands in for it and keeps what `HttpConnector`,
//! `RetryingInputStream` and `CheckContentLengthInputStream` rely on:
//!
//! - a body with `Content-Length` that the server cuts short ends normally (`MeteredStream`
//!   returns -1), so `CheckContentLengthInputStream` reports the mismatch;
//! - a chunked body that ends before its terminating chunk and trailer is
//!   `IOException("Premature EOF")` (`sun.net.www.http.ChunkedInputStream`), so the truncated
//!   bytes are never taken for a complete response. (The `ureq` 2 transport this replaces
//!   returned end of stream for a body cut inside a chunk; review 2026-11-18 round 2, N1.)
//!
//! Each request uses its own connection (no keep-alive pool).

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

use super::http_connector::{HttpTransport, UrlConnection};
use super::proxy_helper::{ProxyInfo, ProxyType};
use super::streams::InputStream;
use blaze_util::string_encoding::{internal_to_unicode, latin1_decode, latin1_encode_lossy};

use crate::errors::{IoException, IoExceptionKind};
use crate::uri::JavaUri;

/// The default transport.
pub struct SocketTransport;

/// Limits on the response head (the JDK has no fixed ones; these only bound memory).
const MAX_HEADER_LINE: usize = 100 * 1024;
const MAX_HEADER_COUNT: usize = 1000;
/// `ChunkedInputStream.MAX_CHUNK_HEADER_SIZE`.
const MAX_CHUNK_HEADER_SIZE: usize = 2050;

enum Connection {
    Plain(TcpStream),
    Tls(Box<StreamOwned<ClientConnection, TcpStream>>),
}

impl Read for Connection {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Connection::Plain(s) => s.read(buf),
            Connection::Tls(s) => s.read(buf),
        }
    }
}

impl Write for Connection {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Connection::Plain(s) => s.write(buf),
            Connection::Tls(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Connection::Plain(s) => s.flush(),
            Connection::Tls(s) => s.flush(),
        }
    }
}

/// The TLS client configuration: the Mozilla root program's CAs (`webpki-roots`), TLS 1.2 and
/// 1.3. See docs/decisions.md (2026-11-18, TLS trust).
fn tls_config() -> Result<Arc<ClientConfig>, IoException> {
    static CONFIG: OnceLock<Result<Arc<ClientConfig>, String>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let roots = RootCertStore {
                roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
            };
            let config = ClientConfig::builder_with_provider(Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .map_err(|e| e.to_string())?
            .with_root_certificates(roots)
            .with_no_client_auth();
            Ok(Arc::new(config))
        })
        .clone()
        .map_err(|m| IoException::of(IoExceptionKind::SSLException, m))
}

/// Maps a socket or TLS error to the Java exception class `HttpConnector` distinguishes.
fn map_io_error(e: &io::Error, connecting: bool) -> IoException {
    if let Some(inner) = e.get_ref() {
        if let Some(e) = inner.downcast_ref::<IoException>() {
            return e.clone();
        }
        if let Some(tls) = inner.downcast_ref::<rustls::Error>() {
            return IoException::of(IoExceptionKind::SSLException, tls.to_string());
        }
    }
    match e.kind() {
        io::ErrorKind::ConnectionRefused => {
            IoException::of(IoExceptionKind::ConnectException, "Connection refused")
        }
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => IoException::of(
            IoExceptionKind::SocketTimeoutException,
            if connecting {
                "Connect timed out"
            } else {
                "Read timed out"
            },
        ),
        io::ErrorKind::ConnectionReset => {
            IoException::of(IoExceptionKind::SocketException, "Connection reset")
        }
        io::ErrorKind::ConnectionAborted | io::ErrorKind::BrokenPipe => {
            IoException::of(IoExceptionKind::SocketException, e.to_string())
        }
        _ => IoException::new(e.to_string()),
    }
}

fn timeout(millis: i32) -> Duration {
    Duration::from_millis(millis.max(1) as u64)
}

/// The host of a URI without the brackets of an IPv6 literal.
fn connect_host(host: &str) -> &str {
    host.strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host)
}

fn connect_tcp(
    host: &str,
    port: u16,
    connect_timeout_ms: i32,
    read_timeout_ms: i32,
) -> Result<TcpStream, IoException> {
    let host = connect_host(host);
    let unknown_host = || IoException::of(IoExceptionKind::UnknownHostException, host.to_string());
    let addrs: Vec<_> = (host, port)
        .to_socket_addrs()
        .map_err(|_| unknown_host())?
        .collect();
    // The JDK connects to one address, `InetAddress.getByName(host)`: the first one in the
    // resolver's order with IPv4 addresses first (`java.net.preferIPv6Addresses` is false by
    // default). It does not try the others.
    let addr = addrs
        .iter()
        .find(|a| a.is_ipv4())
        .or_else(|| addrs.first())
        .ok_or_else(unknown_host)?;
    let stream = TcpStream::connect_timeout(addr, timeout(connect_timeout_ms))
        .map_err(|e| map_io_error(&e, true))?;
    stream
        .set_read_timeout(Some(timeout(read_timeout_ms)))
        .and_then(|()| stream.set_write_timeout(Some(timeout(read_timeout_ms))))
        .map_err(|e| map_io_error(&e, true))?;
    let _ = stream.set_nodelay(true);
    Ok(stream)
}

/// One line of the response head, without its line terminator. `Ok(None)` at end of stream.
fn read_head_line(reader: &mut impl BufRead) -> Result<Option<String>, IoException> {
    let mut line = Vec::new();
    let n = reader
        .by_ref()
        .take(MAX_HEADER_LINE as u64 + 1)
        .read_until(b'\n', &mut line)
        .map_err(|e| map_io_error(&e, false))?;
    if n == 0 {
        return Ok(None);
    }
    if line.len() > MAX_HEADER_LINE {
        return Err(IoException::new("HTTP header line is too long"));
    }
    if line.last() == Some(&b'\n') {
        line.pop();
        if line.last() == Some(&b'\r') {
            line.pop();
        }
    }
    // Header bytes are ISO-8859-1 in Java; one char per byte.
    Ok(Some(line.iter().map(|&b| b as char).collect()))
}

#[derive(Debug)]
struct ResponseHead {
    code: i32,
    message: String,
    /// Lower-cased names, in order.
    headers: Vec<(String, String)>,
}

impl ResponseHead {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .rev()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

fn read_response_head(reader: &mut impl BufRead) -> Result<ResponseHead, IoException> {
    // The JDK: "Unexpected end of file from server" (a SocketException) when the connection
    // closes before a status line.
    let unexpected_eof = || {
        IoException::of(
            IoExceptionKind::SocketException,
            "Unexpected end of file from server",
        )
    };
    let status_line = read_head_line(reader)?.ok_or_else(unexpected_eof)?;
    let mut parts = status_line.splitn(3, ' ');
    let version = parts.next().unwrap_or("");
    // Java `String.trim()` strips chars <= ' ' only (not U+0085 or U+00A0, chars of the
    // Latin-1-decoded line).
    let java_trim = |s: &str| s.trim_matches(|c: char| c <= ' ').to_string();
    let code = parts.next().and_then(|c| java_trim(c).parse::<i32>().ok());
    let (Some(code), true) = (code, version.starts_with("HTTP/")) else {
        return Err(IoException::new(format!(
            "Invalid Http response: {status_line}"
        )));
    };
    let message = java_trim(parts.next().unwrap_or(""));
    // Names with the untrimmed text after the colon; `None` is a line without a colon, which
    // `MessageHeader` keeps under a null key (nothing looks those up).
    let mut raw: Vec<(Option<String>, String)> = Vec::new();
    loop {
        let line = read_head_line(reader)?.ok_or_else(unexpected_eof)?;
        if line.is_empty() {
            break;
        }
        // sun.net.www.MessageHeader.mergeHeader: a line that starts with a character <= ' '
        // continues the previous header; the line break and that first character become one
        // space.
        if line.as_bytes()[0] <= b' '
            && let Some((_, value)) = raw.last_mut()
        {
            value.push(' ');
            value.push_str(&line[1..]);
            continue;
        }
        if raw.len() >= MAX_HEADER_COUNT {
            return Err(IoException::new("Too many HTTP header fields"));
        }
        match line.split_once(':') {
            Some((name, value)) => raw.push((Some(name.to_string()), value.to_string())),
            None => raw.push((None, line)),
        }
    }
    // mergeHeader trims characters <= ' ' around the value (and the name's end).
    let trim = |s: &str| s.trim_matches(|c: char| c <= ' ').to_string();
    let headers = raw
        .into_iter()
        .filter_map(|(name, value)| Some((trim(&name?).to_ascii_lowercase(), trim(&value))))
        .collect();
    Ok(ResponseHead {
        code,
        message,
        headers,
    })
}

/// How the end of the body is found (RFC 9112 section 6.3, as the JDK applies it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BodyFraming {
    /// No body (1xx, 204, 304).
    Empty,
    /// `Content-Length` bytes remain.
    Fixed(u64),
    /// `Transfer-Encoding: chunked`.
    Chunked(ChunkState),
    /// Until the server closes the connection.
    UntilClose,
    /// End of stream was returned.
    Done,
}

/// The position inside a chunked body (`ChunkedInputStream`'s states).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChunkState {
    /// At a chunk-size line.
    AwaitingChunkHeader,
    /// Inside chunk data with this many bytes left.
    ReadingChunk(u64),
    /// At the CRLF that follows chunk data.
    AwaitingChunkEol,
    /// After the last chunk, at the trailer fields.
    AwaitingTrailers,
}

/// The response body with its transfer framing removed.
///
/// PORT: the JDK's `HttpInputStream` over `MeteredStream` (fixed length), `ChunkedInputStream`
/// (chunked) or the socket stream (until close).
pub(crate) struct BodyStream<R: Read + Send> {
    reader: Option<BufReader<R>>,
    framing: BodyFraming,
}

fn premature_eof() -> IoException {
    // sun.net.www.http.ChunkedInputStream.readAheadBlocking: `throw new IOException("Premature
    // EOF")` when the underlying stream ends before the chunked body is complete.
    IoException::new("Premature EOF")
}

impl<R: Read + Send> BodyStream<R> {
    fn new(reader: BufReader<R>, framing: BodyFraming) -> Self {
        BodyStream {
            reader: Some(reader),
            framing,
        }
    }

    /// A chunked body read from `source` (for tests of the framing).
    #[cfg(test)]
    fn chunked(source: R) -> Self {
        BodyStream::new(
            BufReader::new(source),
            BodyFraming::Chunked(ChunkState::AwaitingChunkHeader),
        )
    }

    /// Reads from the connection. A TLS peer that closes without `close_notify` and a reset
    /// after the last byte are reported as end of stream; the framing decides whether the body
    /// was complete.
    fn read_raw(reader: &mut BufReader<R>, buf: &mut [u8]) -> Result<usize, IoException> {
        loop {
            match reader.read(buf) {
                Ok(n) => return Ok(n),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(0),
                Err(e) => return Err(map_io_error(&e, false)),
            }
        }
    }

    /// The bytes up to and including the next LF (`processRaw` scans the raw buffer for
    /// `'\n'`). End of stream before the LF is `Premature EOF`. `max` is the JDK's limit for a
    /// chunk header (`MAX_CHUNK_HEADER_SIZE`); the JDK has none for a trailer line.
    fn read_chunk_line(
        reader: &mut BufReader<R>,
        max: Option<usize>,
    ) -> Result<Vec<u8>, IoException> {
        let mut line = Vec::new();
        loop {
            let mut byte = [0u8; 1];
            if Self::read_raw(reader, &mut byte)? == 0 {
                return Err(premature_eof());
            }
            line.push(byte[0]);
            if byte[0] == b'\n' {
                return Ok(line);
            }
            if max.is_some_and(|max| line.len() >= max) {
                return Err(IoException::new("Chunk header too long"));
            }
        }
    }

    // Java: sun/net/www/http/ChunkedInputStream.java ChunkedInputStream.processRaw (JDK 25.0.2, the embedded JDK; read from the class file)
    fn read_chunked(
        reader: &mut BufReader<R>,
        state: &mut ChunkState,
        buf: &mut [u8],
    ) -> Result<usize, IoException> {
        loop {
            match *state {
                // STATE_AWAITING_CHUNK_HEADER
                ChunkState::AwaitingChunkHeader => {
                    // Find \n to indicate end of chunk header.
                    let header = Self::read_chunk_line(reader, Some(MAX_CHUNK_HEADER_SIZE))?;
                    // Extract the chunk size from the header (ignoring extensions): the
                    // leading hex digits, parsed with Integer.parseInt(header, 0, i, 16).
                    let digits = header.iter().take_while(|b| b.is_ascii_hexdigit()).count();
                    let size = std::str::from_utf8(&header[..digits])
                        .ok()
                        .and_then(|s| i32::from_str_radix(s, 16).ok())
                        .ok_or_else(|| IoException::new("Bogus chunk size"))?;
                    *state = if size > 0 {
                        ChunkState::ReadingChunk(size as u64)
                    } else {
                        ChunkState::AwaitingTrailers
                    };
                }
                // STATE_READING_CHUNK
                ChunkState::ReadingChunk(remaining) => {
                    let want = buf.len().min(remaining.min(usize::MAX as u64) as usize);
                    let n = Self::read_raw(reader, &mut buf[..want])?;
                    if n == 0 {
                        // The connection ended inside a chunk.
                        return Err(premature_eof());
                    }
                    let remaining = remaining - n as u64;
                    *state = if remaining == 0 {
                        ChunkState::AwaitingChunkEol
                    } else {
                        ChunkState::ReadingChunk(remaining)
                    };
                    return Ok(n);
                }
                // STATE_AWAITING_CHUNK_EOL: the two bytes after the chunk are CR LF.
                ChunkState::AwaitingChunkEol => {
                    let mut eol = [0u8; 2];
                    let mut got = 0;
                    while got < 2 {
                        let n = Self::read_raw(reader, &mut eol[got..])?;
                        if n == 0 {
                            return Err(premature_eof());
                        }
                        got += n;
                    }
                    if eol[0] != b'\r' {
                        return Err(IoException::new("missing CR"));
                    }
                    if eol[1] != b'\n' {
                        return Err(IoException::new("missing LF"));
                    }
                    *state = ChunkState::AwaitingChunkHeader;
                }
                // STATE_AWAITING_TRAILERS
                ChunkState::AwaitingTrailers => {
                    let line = Self::read_chunk_line(reader, None)?;
                    // The line without its LF.
                    let line = &line[..line.len() - 1];
                    if line.last() != Some(&b'\r') {
                        return Err(IoException::new("LF should be proceeded by CR"));
                    }
                    // Stream done (an empty line).
                    if line.len() == 1 {
                        return Ok(0);
                    }
                    // PORT: the JDK adds a trailer field to the response headers; the
                    // headers were handed out before the body, and Bazel reads no trailer.
                    if !line.contains(&b':') {
                        return Err(IoException::new(
                            "Malformed tailer - format should be key:value",
                        ));
                    }
                }
            }
        }
    }
}

impl<R: Read + Send> InputStream for BodyStream<R> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, IoException> {
        let Some(reader) = &mut self.reader else {
            return Err(IoException::new("Stream closed"));
        };
        if buf.is_empty() {
            return Ok(0);
        }
        let n = match &mut self.framing {
            BodyFraming::Empty | BodyFraming::Done => 0,
            BodyFraming::Fixed(remaining) => {
                let want = buf.len().min((*remaining).min(usize::MAX as u64) as usize);
                // A short body ends normally (MeteredStream); CheckContentLengthInputStream
                // compares the count with Content-Length.
                let n = Self::read_raw(reader, &mut buf[..want])?;
                *remaining -= n as u64;
                if *remaining == 0 {
                    self.framing = BodyFraming::Done;
                }
                n
            }
            BodyFraming::Chunked(state) => Self::read_chunked(reader, state, buf)?,
            BodyFraming::UntilClose => Self::read_raw(reader, buf)?,
        };
        if n == 0 {
            self.framing = BodyFraming::Done;
        }
        Ok(n)
    }

    fn close(&mut self) -> Result<(), IoException> {
        self.reader = None;
        Ok(())
    }
}

/// The request target and `Host` value of `url`.
fn request_target(url: &JavaUri, absolute: bool) -> String {
    if absolute {
        // Through an HTTP proxy: the absolute form, without the fragment.
        let s = url.as_str();
        return s.split('#').next().unwrap_or(s).to_string();
    }
    let path = match url.get_raw_path() {
        Some(p) if !p.is_empty() => p,
        _ => "/",
    };
    match url.get_raw_query() {
        Some(q) => format!("{path}?{q}"),
        None => path.to_string(),
    }
}

fn host_header(host: &str, port: i32, default_port: u16) -> String {
    if port < 0 || port == default_port as i32 {
        host.to_string()
    } else {
        format!("{host}:{port}")
    }
}

/// The `Proxy-Authorization` value of a `CONNECT` tunnel.
///
/// Java: bazel/repository/downloader/ProxyHelper.java:330-333 hands the credentials to the JDK's
/// `Authenticator` as `internalToUnicode(username)` / `internalToUnicode(password)`, and the
/// JDK's `sun.net.www.protocol.http.BasicAuthentication` encodes `user + ":" + password` as
/// ISO-8859-1 (`?` for a char above U+00FF). Checked on the oracle's JDK (Zulu 25.0.2): user `é`,
/// password `pw` gives `Basic 6Tpwdw==`. Plain-HTTP proxying uses the request property set by
/// `HttpConnector` instead (`basic_http_authentication_encode`, the internal bytes).
fn proxy_authorization(proxy: &ProxyInfo) -> Option<String> {
    use base64::Engine as _;
    match (proxy.username(), proxy.password()) {
        (Some(u), Some(p)) => {
            let plain = format!("{}:{}", internal_to_unicode(u), internal_to_unicode(p));
            Some(format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD.encode(latin1_encode_lossy(&plain))
            ))
        }
        _ => None,
    }
}

/// Opens a tunnel to `host:port` through an HTTP proxy (`CONNECT`).
fn connect_tunnel(
    stream: TcpStream,
    proxy: &ProxyInfo,
    host: &str,
    port: u16,
) -> Result<TcpStream, IoException> {
    let mut request = format!("CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\n");
    if let Some(auth) = proxy_authorization(proxy) {
        request.push_str(&format!("Proxy-Authorization: {auth}\r\n"));
    }
    request.push_str("\r\n");
    // The JDK writes requests through a `PrintStream` with the default charset (ISO-8859-1).
    (&stream)
        .write_all(&latin1_encode_lossy(&request))
        .map_err(|e| map_io_error(&e, false))?;
    // Read the proxy's answer byte by byte so that nothing of the tunnelled stream is consumed.
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        match (&stream).read(&mut byte) {
            Ok(0) => {
                return Err(IoException::of(
                    IoExceptionKind::SocketException,
                    "Unexpected end of file from server",
                ));
            }
            Ok(_) => head.push(byte[0]),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(map_io_error(&e, false)),
        }
        if head.len() > MAX_HEADER_LINE {
            return Err(IoException::new("HTTP header line is too long"));
        }
    }
    // Response header bytes are read one char per byte (`MessageHeader.parseHeader`).
    let status_line = latin1_decode(&head);
    let status_line = status_line.lines().next().unwrap_or("");
    let code = status_line.split(' ').nth(1).unwrap_or("");
    if code != "200" {
        // sun.net.www.protocol.http.HttpURLConnection.doTunneling
        return Err(IoException::new(format!(
            "Unable to tunnel through proxy. Proxy returns \"{status_line}\""
        )));
    }
    Ok(stream)
}

/// The last line of defence for what is written into the request head: `HttpConnector` applies
/// the JDK's `checkMessageHeader` to caller headers (`RequestProperties`), and the transport
/// applies it again to everything it is given, so that no header can end the header block or
/// start another line.
///
/// PORT: stricter than the JDK in two points, both outside what `HttpConnector` can pass on:
/// a carriage return in a name and a carriage return not followed by a line feed in a value are
/// refused (the JDK writes them as they are; some servers take a bare CR for a line end).
fn check_request_header(name: &str, value: &str) -> Result<(), IoException> {
    let illegal = |message: String| {
        Err(IoException::of(
            IoExceptionKind::UnrecoverableHttpException,
            message,
        ))
    };
    if let Err(e) = super::http_connector::check_message_header(name, value) {
        return illegal(e.0);
    }
    if name.is_empty() || name.contains('\r') {
        return illegal(format!(
            "Illegal character(s) in message header field: {name}"
        ));
    }
    let bytes = value.as_bytes();
    if bytes
        .iter()
        .enumerate()
        .any(|(i, b)| *b == b'\r' && bytes.get(i + 1) != Some(&b'\n'))
    {
        return illegal(format!(
            "Illegal character(s) in message header value: {value}"
        ));
    }
    Ok(())
}

/// The request head: the request line, the caller's headers in their order, then what
/// `HttpURLConnection.writeRequests` adds when the caller has not set it (`setIfNotSet`): `Host`,
/// `Accept` and `Connection: keep-alive` (`Proxy-Connection` through a proxy without a tunnel).
///
/// `Host` always comes from the URL: `HttpURLConnection` does not let a caller set it
/// (`isRestrictedHeader`), and a transport that sent a caller's `Host` would let a request reach
/// another virtual host than the one its URL names. A `Host` among `headers` is an error here.
// Java: sun/net/www/protocol/http/HttpURLConnection.java HttpURLConnection.writeRequests (JDK 25.0.2)
fn request_head(
    url: &JavaUri,
    host: &str,
    default_port: u16,
    headers: &[(String, String)],
    through_proxy_without_tunnel: bool,
) -> Result<String, IoException> {
    let has = |name: &str| headers.iter().any(|(k, _)| k.eq_ignore_ascii_case(name));
    if has("host") {
        return Err(IoException::of(
            IoExceptionKind::UnrecoverableHttpException,
            "the Host header is set from the URL and cannot be overridden",
        ));
    }
    let mut request = format!(
        "GET {} HTTP/1.1\r\n",
        request_target(url, through_proxy_without_tunnel)
    );
    for (name, value) in headers {
        check_request_header(name, value)?;
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str(&format!(
        "Host: {}\r\n",
        host_header(host, url.get_port(), default_port)
    ));
    if !has("accept") {
        request.push_str("Accept: */*\r\n");
    }
    // The connection is not reused: it is dropped with the body stream.
    if through_proxy_without_tunnel {
        if !has("proxy-connection") {
            request.push_str("Proxy-Connection: keep-alive\r\n");
        }
    } else if !has("connection") {
        request.push_str("Connection: keep-alive\r\n");
    }
    request.push_str("\r\n");
    Ok(request)
}

/// How the body of a response is framed.
// Java: sun/net/www/http/HttpClient.java HttpClient.parseHTTPHeader (JDK 25.0.2)
fn body_framing(head: &ResponseHead) -> BodyFraming {
    if head.code == 204 || head.code == 304 {
        return BodyFraming::Empty;
    }
    // "Transfer-Encoding: chunked" decides, whatever the HTTP version of the response.
    if head
        .header("transfer-encoding")
        .is_some_and(|te| te.eq_ignore_ascii_case("chunked"))
    {
        return BodyFraming::Chunked(ChunkState::AwaitingChunkHeader);
    }
    match head
        .header("content-length")
        .and_then(|l| l.trim().parse::<u64>().ok())
    {
        Some(0) => BodyFraming::Empty,
        Some(len) => BodyFraming::Fixed(len),
        None => BodyFraming::UntilClose,
    }
}

/// Opens the connection, sends the request and reads the response head. `tls` replaces the
/// default TLS configuration (tests only).
fn open_connection(
    url: &JavaUri,
    proxy: &ProxyInfo,
    headers: &[(String, String)],
    connect_timeout_ms: i32,
    read_timeout_ms: i32,
    tls: Option<Arc<ClientConfig>>,
) -> Result<UrlConnection, IoException> {
    let https = match url.get_scheme().map(str::to_ascii_lowercase).as_deref() {
        Some("http") => false,
        Some("https") => true,
        other => {
            return Err(IoException::of(
                IoExceptionKind::UnrecoverableHttpException,
                format!("unknown protocol: {}", other.unwrap_or("")),
            ));
        }
    };
    let host = url.get_host().ok_or_else(|| {
        IoException::of(
            IoExceptionKind::UnrecoverableHttpException,
            format!("no host in URL: {}", url.as_str()),
        )
    })?;
    let default_port: u16 = if https { 443 } else { 80 };
    let port: u16 = match url.get_port() {
        p if p < 0 => default_port,
        p => u16::try_from(p).map_err(|_| {
            IoException::of(
                IoExceptionKind::UnrecoverableHttpException,
                format!("port out of range: {p}"),
            )
        })?,
    };
    // Built before anything is connected: a bad header fails without a request.
    let through_proxy_without_tunnel = proxy.proxy_type == ProxyType::HTTP && !https;
    let request = request_head(
        url,
        host,
        default_port,
        headers,
        through_proxy_without_tunnel,
    )?;

    let tcp = match proxy.proxy_type {
        ProxyType::DIRECT => connect_tcp(host, port, connect_timeout_ms, read_timeout_ms)?,
        ProxyType::HTTP => {
            let proxy_host = proxy.hostname.as_deref().unwrap_or("");
            let proxy_port = u16::try_from(proxy.port).map_err(|_| {
                IoException::new(format!("proxy port out of range: {}", proxy.port))
            })?;
            let stream = connect_tcp(proxy_host, proxy_port, connect_timeout_ms, read_timeout_ms)?;
            if https {
                connect_tunnel(stream, proxy, host, port)?
            } else {
                // The absolute-form request goes to the proxy; `HttpConnector` has added
                // `Proxy-Authorization` to the request properties.
                stream
            }
        }
        // PORT: java.net.SocksSocketImpl is not ported.
        ProxyType::SOCKS => blaze_util::unported!(
            "SocksSocketImpl.connect",
            "SOCKS proxy {}",
            proxy.address().unwrap_or_default()
        ),
    };

    let mut connection = if https {
        let config = match tls {
            Some(config) => config,
            None => tls_config()?,
        };
        let server_name = ServerName::try_from(connect_host(host).to_string())
            .map_err(|e| IoException::of(IoExceptionKind::SSLException, e.to_string()))?;
        let client = ClientConnection::new(config, server_name)
            .map_err(|e| IoException::of(IoExceptionKind::SSLException, e.to_string()))?;
        Connection::Tls(Box::new(StreamOwned::new(client, tcp)))
    } else {
        Connection::Plain(tcp)
    };

    // sun.net.www.http.HttpClient writes the request through a `PrintStream` with
    // `NetworkClient.encoding`, the default charset (ISO-8859-1): header values are internal
    // strings and leave as their bytes.
    connection
        .write_all(&latin1_encode_lossy(&request))
        .and_then(|()| connection.flush())
        .map_err(|e| map_io_error(&e, false))?;

    let mut reader = BufReader::new(connection);
    let head = loop {
        let head = read_response_head(&mut reader)?;
        if head.code == 101 {
            // HttpClient.parseHTTPHeader: a ProtocolException.
            return Err(IoException::new("Unexpected 101 response from server"));
        }
        // "Ignoring interim informational 1xx response": the final response follows.
        if !(100..200).contains(&head.code) {
            break head;
        }
    };
    let framing = body_framing(&head);
    Ok(UrlConnection::new(
        url.clone(),
        head.code,
        Some(head.message),
        head.headers,
        Ok(Box::new(BodyStream::new(reader, framing))),
    ))
}

impl HttpTransport for SocketTransport {
    fn open(
        &self,
        url: &JavaUri,
        proxy: &ProxyInfo,
        headers: &[(String, String)],
        connect_timeout_ms: i32,
        read_timeout_ms: i32,
    ) -> Result<UrlConnection, IoException> {
        open_connection(
            url,
            proxy,
            headers,
            connect_timeout_ms,
            read_timeout_ms,
            None,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Byte-strings review 2026-11-20 S1-4. Expected values from the oracle's JDK (Zulu 25.0.2,
    /// `-Dfile.encoding=ISO-8859-1`): an `HttpURLConnection` through an HTTP proxy, after a 407,
    /// sends `Proxy-authorization: Basic 6Tpwdw==` for the `Authenticator` user `é` (U+00E9,
    /// what `internalToUnicode` makes of C3 A9) and password `pw`. A char above U+00FF is `?`
    /// (`Basic Pzpwdw==` for `中`).
    #[test]
    fn connect_tunnel_credentials_are_latin1_of_the_unicode_string() {
        use blaze_util::string_encoding::latin1_decode;
        let proxy = |url: &str| {
            super::super::proxy_helper::create_proxy(Some(&latin1_decode(url.as_bytes()))).unwrap()
        };
        assert_eq!(
            proxy_authorization(&proxy("http://é:pw@127.0.0.1:3128")).unwrap(),
            "Basic 6Tpwdw=="
        );
        assert_eq!(
            proxy_authorization(&proxy("http://中:pw@127.0.0.1:3128")).unwrap(),
            "Basic Pzpwdw=="
        );
        assert_eq!(
            proxy_authorization(&proxy("http://user:pass@127.0.0.1:3128")).unwrap(),
            "Basic dXNlcjpwYXNz"
        );
    }

    fn read_all(body: &mut BodyStream<&'static [u8]>) -> Result<Vec<u8>, IoException> {
        let mut out = Vec::new();
        let mut buf = [0u8; 5];
        loop {
            let n = body.read(&mut buf)?;
            if n == 0 {
                return Ok(out);
            }
            out.extend_from_slice(&buf[..n]);
        }
    }

    fn chunked(raw: &'static [u8]) -> Result<Vec<u8>, IoException> {
        read_all(&mut BodyStream::chunked(raw))
    }

    #[test]
    fn chunked_body_is_decoded() {
        assert_eq!(
            chunked(b"3\r\nhel\r\nb;ext=1\r\nlo world!!!\r\n0\r\n\r\n").unwrap(),
            b"hello world!!!"
        );
        // Trailer fields are skipped.
        assert_eq!(
            chunked(b"5\r\nhello\r\n0\r\nX-Trailer: 1\r\n\r\n").unwrap(),
            b"hello"
        );
        assert_eq!(chunked(b"0\r\n\r\n").unwrap(), b"");
    }

    /// `ChunkedInputStream`: every way a chunked body can stop early is "Premature EOF".
    #[test]
    fn truncated_chunked_body_is_premature_eof() {
        for raw in [
            // inside a chunk
            &b"a\r\nhello"[..],
            // after a chunk's data, before its CRLF
            b"5\r\nhello",
            // after a complete chunk, before the next chunk-size line
            b"5\r\nhello\r\n",
            // inside a chunk-size line
            b"5\r\nhello\r\n1",
            // after the last chunk, before the end of the trailer
            b"5\r\nhello\r\n0\r\n",
            b"5\r\nhello\r\n0\r\nX-Trailer: 1\r\n",
            // nothing at all
            b"",
        ] {
            let e = chunked(raw).unwrap_err();
            assert_eq!(e.kind, IoExceptionKind::IOException, "{raw:?}");
            assert_eq!(e.get_message(), "Premature EOF", "{raw:?}");
        }
    }

    /// `ChunkedInputStream.processRaw`, state by state.
    #[test]
    fn chunk_framing_follows_the_jdk() {
        let message = |raw: &'static [u8]| chunked(raw).unwrap_err().get_message();
        // Chunk header: the leading hex digits up to the LF; a CR is not required, and what
        // follows the digits (an extension, anything) is ignored.
        assert_eq!(chunked(b"5\nhello\r\n0\r\n\r\n").unwrap(), b"hello");
        assert_eq!(chunked(b"5 ; ext\r\nhello\r\n0\r\n\r\n").unwrap(), b"hello");
        assert_eq!(chunked(b"5xyz\r\nhello\r\n0\r\n\r\n").unwrap(), b"hello");
        // Integer.parseInt of no digits, or of more than an int, is a NumberFormatException.
        assert_eq!(message(b"zz\r\nhello\r\n0\r\n\r\n"), "Bogus chunk size");
        assert_eq!(message(b"\r\nhello\r\n0\r\n\r\n"), "Bogus chunk size");
        assert_eq!(message(b" 5\r\nhello\r\n0\r\n\r\n"), "Bogus chunk size");
        assert_eq!(message(b"80000000\r\n"), "Bogus chunk size");
        // After the chunk's data: CR, then LF.
        assert_eq!(message(b"5\r\nhelloXX\r\n0\r\n\r\n"), "missing CR");
        assert_eq!(message(b"5\r\nhello\rX0\r\n\r\n"), "missing LF");
        // Trailer lines end in CR LF and are `key:value`.
        assert_eq!(
            message(b"5\r\nhello\r\n0\r\n\n"),
            "LF should be proceeded by CR"
        );
        assert_eq!(
            message(b"5\r\nhello\r\n0\r\nX-Trailer: 1\n\r\n"),
            "LF should be proceeded by CR"
        );
        assert_eq!(
            message(b"5\r\nhello\r\n0\r\nnot a trailer\r\n\r\n"),
            "Malformed tailer - format should be key:value"
        );
    }

    /// `MAX_CHUNK_HEADER_SIZE`: a chunk header without a line feed in 2050 bytes.
    #[test]
    fn chunk_header_too_long() {
        let raw: &'static [u8] = Box::leak(vec![b'1'; 4000].into_boxed_slice());
        assert_eq!(
            chunked(raw).unwrap_err().get_message(),
            "Chunk header too long"
        );
    }

    /// `MeteredStream`: a body shorter than `Content-Length` ends normally.
    #[test]
    fn short_fixed_length_body_ends_normally() {
        let mut body = BodyStream::new(BufReader::new(&b"short"[..]), BodyFraming::Fixed(10));
        assert_eq!(read_all(&mut body).unwrap(), b"short");
        // And a longer one stops at Content-Length.
        let mut body = BodyStream::new(
            BufReader::new(&b"0123456789tail"[..]),
            BodyFraming::Fixed(10),
        );
        assert_eq!(read_all(&mut body).unwrap(), b"0123456789");
    }

    #[test]
    fn response_head_is_parsed() {
        let mut raw =
            &b"HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nX-A:  b \r\n\r\nbody"[..];
        let head = read_response_head(&mut raw).unwrap();
        assert_eq!(head.code, 404);
        assert_eq!(head.message, "Not Found");
        assert_eq!(
            head.headers,
            vec![
                ("content-type".to_string(), "text/plain".to_string()),
                ("x-a".to_string(), "b".to_string()),
            ]
        );
        assert_eq!(raw, b"body");

        let e = read_response_head(&mut &b""[..]).unwrap_err();
        assert_eq!(e.kind, IoExceptionKind::SocketException);
        assert_eq!(e.get_message(), "Unexpected end of file from server");
    }

    /// Byte-strings review 2026-11-20 S2-6: the status line is Latin-1 text and its parts are
    /// trimmed as Java's `String.trim()` does (chars <= ' '), which keeps U+0085 and U+00A0.
    #[test]
    fn status_line_trim_keeps_latin1_spaces() {
        let mut raw = &b"HTTP/1.1 200 OK\xa0\x85\r\n\r\n"[..];
        let head = read_response_head(&mut raw).unwrap();
        assert_eq!(head.code, 200);
        assert_eq!(head.message, "OK\u{a0}\u{85}");
    }

    #[test]
    fn request_target_and_host() {
        let url = JavaUri::create("http://example.com:8080/a%20b/c?x=1#frag").unwrap();
        assert_eq!(request_target(&url, false), "/a%20b/c?x=1");
        assert_eq!(
            request_target(&url, true),
            "http://example.com:8080/a%20b/c?x=1"
        );
        assert_eq!(host_header("example.com", 8080, 80), "example.com:8080");
        assert_eq!(host_header("example.com", -1, 80), "example.com");
        assert_eq!(host_header("example.com", 443, 443), "example.com");
        let url = JavaUri::create("https://example.com").unwrap();
        assert_eq!(request_target(&url, false), "/");
    }

    // ---------------------------------------------------------------------------------------
    // Response head (review 2026-11-18 round 3, R3-3, R3-5)
    // ---------------------------------------------------------------------------------------

    /// `MessageHeader.mergeHeader`: a continuation line is appended to the previous header with
    /// one space for the line break and its first character.
    #[test]
    fn folded_response_headers_are_merged() {
        let mut raw = &b"HTTP/1.1 200 OK\r\nX-A: one\r\n\ttwo: 2\r\n  three\r\nX-B:\r\n b\r\nno colon\r\nX-C: c \r\n\r\n"[..];
        let head = read_response_head(&mut raw).unwrap();
        assert_eq!(
            head.headers,
            vec![
                ("x-a".to_string(), "one two: 2  three".to_string()),
                ("x-b".to_string(), "b".to_string()),
                ("x-c".to_string(), "c".to_string()),
            ]
        );
    }

    fn head_of(raw: &'static [u8]) -> ResponseHead {
        read_response_head(&mut &raw[..]).unwrap()
    }

    /// `HttpClient.parseHTTPHeader`: chunked wins over `Content-Length`, whatever the version.
    #[test]
    fn body_framing_follows_the_jdk() {
        let chunked = BodyFraming::Chunked(ChunkState::AwaitingChunkHeader);
        assert_eq!(
            body_framing(&head_of(
                b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nTransfer-Encoding: Chunked\r\n\r\n"
            )),
            chunked
        );
        assert_eq!(
            body_framing(&head_of(
                b"HTTP/1.0 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n"
            )),
            chunked
        );
        // Not exactly "chunked": not chunked.
        assert_eq!(
            body_framing(&head_of(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: gzip, chunked\r\nContent-Length: 5\r\n\r\n"
            )),
            BodyFraming::Fixed(5)
        );
        assert_eq!(
            body_framing(&head_of(b"HTTP/1.1 200 OK\r\n\r\n")),
            BodyFraming::UntilClose
        );
        assert_eq!(
            body_framing(&head_of(
                b"HTTP/1.1 304 Not Modified\r\nContent-Length: 5\r\n\r\n"
            )),
            BodyFraming::Empty
        );
    }

    // ---------------------------------------------------------------------------------------
    // Request head (review 2026-11-18 round 3, R3-1, R3-6, R3-9)
    // ---------------------------------------------------------------------------------------

    fn header(name: &str, value: &str) -> (String, String) {
        (name.to_string(), value.to_string())
    }

    /// `HttpURLConnection.writeRequests`: the caller's properties, then Host, Accept and
    /// Connection where not set. Repeated properties are separate lines.
    #[test]
    fn request_head_is_written_as_the_jdk_writes_it() {
        let url = JavaUri::create("http://example.com:8080/a?b=c").unwrap();
        let headers = [
            header("Accept-Encoding", "gzip"),
            header("X-Multi", "1"),
            header("X-Multi", "2"),
            header("User-Agent", "bazel/test"),
        ];
        assert_eq!(
            request_head(&url, "example.com", 80, &headers, false).unwrap(),
            "GET /a?b=c HTTP/1.1\r\n\
             Accept-Encoding: gzip\r\n\
             X-Multi: 1\r\n\
             X-Multi: 2\r\n\
             User-Agent: bazel/test\r\n\
             Host: example.com:8080\r\n\
             Accept: */*\r\n\
             Connection: keep-alive\r\n\
             \r\n"
        );
        // Through a proxy without a tunnel: the absolute form and Proxy-Connection. The proxy
        // credentials are a request property set by HttpConnector, written once.
        let headers = [
            header("Proxy-Authorization", "Basic dXNlcjpwYXNz"),
            header("Accept", "text/plain"),
            header("Connection", "close"),
        ];
        assert_eq!(
            request_head(&url, "example.com", 80, &headers, true).unwrap(),
            "GET http://example.com:8080/a?b=c HTTP/1.1\r\n\
             Proxy-Authorization: Basic dXNlcjpwYXNz\r\n\
             Accept: text/plain\r\n\
             Connection: close\r\n\
             Host: example.com:8080\r\n\
             Proxy-Connection: keep-alive\r\n\
             \r\n"
        );
    }

    /// The transport's own check of what it writes: nothing can end the header block early,
    /// and `Host` is never taken from the caller.
    #[test]
    fn request_head_refuses_header_injection() {
        let url = JavaUri::create("http://example.com/").unwrap();
        let error = |name: &str, value: &str| {
            let e =
                request_head(&url, "example.com", 80, &[header(name, value)], false).unwrap_err();
            assert_eq!(e.kind, IoExceptionKind::UnrecoverableHttpException);
            e.get_message()
        };
        // The JDK's two messages (checkMessageHeader).
        assert_eq!(
            error("Authorization", "Bearer tok\n"),
            "Illegal character(s) in message header value: Bearer tok\n"
        );
        assert_eq!(
            error("Authorization", "Bearer tok\r\nX-Injected: 1"),
            "Illegal character(s) in message header value: Bearer tok\r\nX-Injected: 1"
        );
        assert_eq!(
            error("X-A\nX-B", "v"),
            "Illegal character(s) in message header field: X-A\nX-B"
        );
        assert_eq!(
            error("X-A: v\r\nX-B", "v"),
            "Illegal character(s) in message header field: X-A: v\r\nX-B"
        );
        // Beyond the JDK: a bare carriage return.
        assert_eq!(
            error("X-A\rX-B", "v"),
            "Illegal character(s) in message header field: X-A\rX-B"
        );
        assert_eq!(
            error("X-A", "v\rX-B: 1"),
            "Illegal character(s) in message header value: v\rX-B: 1"
        );
        assert_eq!(
            error("hOsT", "other.example"),
            "the Host header is set from the URL and cannot be overridden"
        );
        // A continuation line is legal for the JDK and stays inside its header.
        assert!(
            request_head(
                &url,
                "example.com",
                80,
                &[header("X-A", "one\n two")],
                false
            )
            .unwrap()
            .contains("X-A: one\n two\r\nHost: example.com\r\n")
        );
    }

    // ---------------------------------------------------------------------------------------
    // TLS and proxies, against local servers (review 2026-11-18 round 3, R3-2)
    // ---------------------------------------------------------------------------------------
    //
    // testdata/: a throwaway CA (its private key was deleted after signing) and a certificate
    // for `localhost` signed by it, valid until 2126. The CA is trusted by nothing but the
    // client configuration these tests build.

    use std::net::{Shutdown, TcpListener};
    use std::sync::Mutex;

    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

    const TEST_CA: &[u8] = include_bytes!("testdata/test_ca.der");
    const TEST_SERVER_CERT: &[u8] = include_bytes!("testdata/test_server.der");
    const TEST_SERVER_KEY: &[u8] = include_bytes!("testdata/test_server_key.pk8");

    /// A client configuration that trusts the test CA only.
    fn test_ca_client_config() -> Arc<ClientConfig> {
        let mut roots = RootCertStore::empty();
        roots.add(CertificateDer::from(TEST_CA.to_vec())).unwrap();
        Arc::new(
            ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_root_certificates(roots)
                .with_no_client_auth(),
        )
    }

    fn read_head(stream: &mut impl Read) -> String {
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            match stream.read(&mut byte) {
                Ok(1) => head.push(byte[0]),
                _ => break,
            }
        }
        String::from_utf8_lossy(&head).into_owned()
    }

    /// An HTTPS server on 127.0.0.1 with the `localhost` certificate: answers every request
    /// with `response`. Returns the port and the request heads received over TLS.
    fn tls_server(response: &'static str) -> (u16, Arc<Mutex<Vec<String>>>) {
        let config = Arc::new(
            rustls::ServerConfig::builder_with_provider(Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(TEST_SERVER_CERT.to_vec())],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(TEST_SERVER_KEY.to_vec())),
            )
            .unwrap(),
        );
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        std::thread::spawn(move || {
            for socket in listener.incoming() {
                let Ok(socket) = socket else { return };
                let Ok(connection) = rustls::ServerConnection::new(config.clone()) else {
                    continue;
                };
                let mut tls = StreamOwned::new(connection, socket);
                let head = read_head(&mut tls);
                if head.is_empty() {
                    // The handshake failed (the client did not accept the certificate).
                    continue;
                }
                recorded.lock().unwrap().push(head);
                let _ = tls.write_all(response.as_bytes());
                tls.conn.send_close_notify();
                let _ = tls.flush();
                let _ = tls.sock.shutdown(Shutdown::Both);
            }
        });
        (port, requests)
    }

    fn read_body(mut connection: UrlConnection) -> Result<Vec<u8>, IoException> {
        let mut stream = connection.get_input_stream()?;
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

    const OK_CHUNKED: &str =
        "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n6\r\nsecure\r\n0\r\n\r\n";

    fn open_test(
        url: &str,
        proxy: &ProxyInfo,
        headers: &[(String, String)],
        tls: Option<Arc<ClientConfig>>,
    ) -> Result<UrlConnection, IoException> {
        open_connection(
            &JavaUri::create(url).unwrap(),
            proxy,
            headers,
            5000,
            5000,
            tls,
        )
    }

    #[test]
    fn https_request_to_a_server_with_a_trusted_certificate() {
        let (port, requests) = tls_server(OK_CHUNKED);
        let connection = open_test(
            &format!("https://localhost:{port}/a/b?c=d"),
            &ProxyInfo::no_proxy(),
            &[header("User-Agent", "bazel/test")],
            Some(test_ca_client_config()),
        )
        .unwrap();
        assert_eq!(connection.get_response_code(), 200);
        assert_eq!(read_body(connection).unwrap(), b"secure");
        let requests = requests.lock().unwrap().clone();
        assert_eq!(requests.len(), 1);
        assert!(
            requests[0].starts_with("GET /a/b?c=d HTTP/1.1\r\nUser-Agent: bazel/test\r\n"),
            "{requests:?}"
        );
        assert!(
            requests[0].contains(&format!("\r\nHost: localhost:{port}\r\n")),
            "{requests:?}"
        );
    }

    /// The default configuration (the public CA roots) does not trust the test CA: the
    /// handshake fails with an SSLException that names the certificate, and no request is sent,
    /// neither over TLS nor in the clear.
    #[test]
    fn https_request_to_a_server_with_an_untrusted_certificate_fails() {
        let (port, requests) = tls_server(OK_CHUNKED);
        let e = open_test(
            &format!("https://localhost:{port}/"),
            &ProxyInfo::no_proxy(),
            &[],
            None,
        )
        .err()
        .expect("the certificate must be rejected");
        assert_eq!(e.kind, IoExceptionKind::SSLException, "{e:?}");
        // HttpConnector treats it as permanent by this word.
        assert!(e.get_message().contains("certificate"), "{e:?}");
        assert!(requests.lock().unwrap().is_empty());
    }

    /// The certificate is for `localhost`; the same server reached by another name is refused.
    #[test]
    fn https_request_with_a_certificate_for_another_name_fails() {
        let (port, requests) = tls_server(OK_CHUNKED);
        let e = open_test(
            &format!("https://127.0.0.1:{port}/"),
            &ProxyInfo::no_proxy(),
            &[],
            Some(test_ca_client_config()),
        )
        .err()
        .expect("the name must be checked");
        assert_eq!(e.kind, IoExceptionKind::SSLException, "{e:?}");
        assert!(e.get_message().contains("certificate"), "{e:?}");
        assert!(requests.lock().unwrap().is_empty());
    }

    /// A plain-HTTP server on an https URL: no fallback to the clear.
    #[test]
    fn https_request_to_a_plain_http_server_fails() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let recorded = seen.clone();
        std::thread::spawn(move || {
            for socket in listener.incoming() {
                let Ok(mut socket) = socket else { return };
                let mut buf = [0u8; 16];
                let n = socket.read(&mut buf).unwrap_or(0);
                recorded.lock().unwrap().push(buf[..n].to_vec());
                let _ = socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok");
            }
        });
        let e = open_test(
            &format!("https://localhost:{port}/"),
            &ProxyInfo::no_proxy(),
            &[],
            Some(test_ca_client_config()),
        )
        .err()
        .expect("TLS must fail");
        assert_eq!(e.kind, IoExceptionKind::SSLException, "{e:?}");
        // What the server saw is a TLS ClientHello, not an HTTP request.
        let seen = seen.lock().unwrap().clone();
        assert!(
            seen.iter().all(|bytes| !bytes.starts_with(b"GET ")),
            "{seen:?}"
        );
    }

    /// An HTTP proxy on 127.0.0.1. `CONNECT` requests are answered with `connect_status` and,
    /// for a 200, tunnelled to the named port on 127.0.0.1; other requests are answered with
    /// "proxied". Returns the port and the request heads the proxy received.
    fn proxy_server(connect_status: &'static str) -> (u16, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        std::thread::spawn(move || {
            for socket in listener.incoming() {
                let Ok(mut client) = socket else { return };
                let head = read_head(&mut client);
                recorded.lock().unwrap().push(head.clone());
                let Some(target) = head
                    .strip_prefix("CONNECT ")
                    .and_then(|rest| rest.split(' ').next())
                else {
                    let _ =
                        client.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\n\r\nproxied");
                    continue;
                };
                let _ = client.write_all(format!("HTTP/1.1 {connect_status}\r\n\r\n").as_bytes());
                if !connect_status.starts_with("200") {
                    continue;
                }
                let target_port: u16 = target.rsplit(':').next().unwrap().parse().unwrap();
                let Ok(mut upstream) = TcpStream::connect(("127.0.0.1", target_port)) else {
                    continue;
                };
                let (mut client_read, mut upstream_write) =
                    (client.try_clone().unwrap(), upstream.try_clone().unwrap());
                std::thread::spawn(move || {
                    let _ = std::io::copy(&mut client_read, &mut upstream_write);
                    let _ = upstream_write.shutdown(Shutdown::Write);
                });
                std::thread::spawn(move || {
                    let _ = std::io::copy(&mut upstream, &mut client);
                    let _ = client.shutdown(Shutdown::Write);
                });
            }
        });
        (port, requests)
    }

    fn proxy_info(port: u16, credentials: &str) -> ProxyInfo {
        super::super::proxy_helper::create_proxy(Some(&format!(
            "http://{credentials}127.0.0.1:{port}"
        )))
        .unwrap()
    }

    /// https through an authenticated proxy: `CONNECT` with the proxy credentials, then TLS to
    /// the origin inside the tunnel. The credentials do not go to the origin, and the origin's
    /// request does not go to the proxy.
    #[test]
    fn https_request_through_a_proxy_uses_a_connect_tunnel() {
        let (origin_port, origin_requests) = tls_server(OK_CHUNKED);
        let (proxy_port, proxy_requests) = proxy_server("200 Connection established");
        let connection = open_test(
            &format!("https://localhost:{origin_port}/secret/path"),
            &proxy_info(proxy_port, "user:pass@"),
            &[header("Authorization", "Bearer origin-token")],
            Some(test_ca_client_config()),
        )
        .unwrap();
        assert_eq!(read_body(connection).unwrap(), b"secure");

        let proxy_requests = proxy_requests.lock().unwrap().clone();
        assert_eq!(
            proxy_requests,
            vec![format!(
                "CONNECT localhost:{origin_port} HTTP/1.1\r\n\
                 Host: localhost:{origin_port}\r\n\
                 Proxy-Authorization: Basic dXNlcjpwYXNz\r\n\r\n"
            )]
        );
        let origin_requests = origin_requests.lock().unwrap().clone();
        assert_eq!(origin_requests.len(), 1);
        assert!(
            origin_requests[0]
                .starts_with("GET /secret/path HTTP/1.1\r\nAuthorization: Bearer origin-token\r\n"),
            "{origin_requests:?}"
        );
        assert!(
            !origin_requests[0].to_ascii_lowercase().contains("proxy-"),
            "{origin_requests:?}"
        );
    }

    /// The certificate is still checked against the origin's name through the tunnel.
    #[test]
    fn https_request_through_a_proxy_still_verifies_the_origin() {
        let (origin_port, origin_requests) = tls_server(OK_CHUNKED);
        let (proxy_port, _) = proxy_server("200 Connection established");
        let e = open_test(
            &format!("https://localhost:{origin_port}/"),
            &proxy_info(proxy_port, ""),
            &[],
            None,
        )
        .err()
        .expect("the certificate must be rejected");
        assert_eq!(e.kind, IoExceptionKind::SSLException, "{e:?}");
        assert!(origin_requests.lock().unwrap().is_empty());
    }

    // Java: sun/net/www/protocol/http/HttpURLConnection.java HttpURLConnection.doTunneling
    #[test]
    fn proxy_that_refuses_the_tunnel_is_an_error() {
        let (origin_port, origin_requests) = tls_server(OK_CHUNKED);
        let (proxy_port, _) = proxy_server("407 Proxy Authentication Required");
        let e = open_test(
            &format!("https://localhost:{origin_port}/"),
            &proxy_info(proxy_port, ""),
            &[],
            Some(test_ca_client_config()),
        )
        .err()
        .expect("the tunnel must fail");
        assert_eq!(
            e.get_message(),
            "Unable to tunnel through proxy. Proxy returns \"HTTP/1.1 407 Proxy Authentication \
             Required\""
        );
        assert!(origin_requests.lock().unwrap().is_empty());
    }

    /// http through a proxy: the absolute form, with the `Proxy-Authorization` that
    /// `HttpConnector` set as a request property, once.
    #[test]
    fn http_request_through_a_proxy_uses_the_absolute_form() {
        let (proxy_port, proxy_requests) = proxy_server("200 Connection established");
        let proxy = proxy_info(proxy_port, "user:pass@");
        let connection = open_test(
            "http://origin.invalid:8080/x?y=1",
            &proxy,
            &[header(
                "Proxy-Authorization",
                &proxy.get_proxy_authorization_header().unwrap(),
            )],
            None,
        )
        .unwrap();
        assert_eq!(read_body(connection).unwrap(), b"proxied");
        assert_eq!(
            proxy_requests.lock().unwrap().clone(),
            vec![
                "GET http://origin.invalid:8080/x?y=1 HTTP/1.1\r\n\
                 Proxy-Authorization: Basic dXNlcjpwYXNz\r\n\
                 Host: origin.invalid:8080\r\n\
                 Accept: */*\r\n\
                 Proxy-Connection: keep-alive\r\n\r\n"
                    .to_string()
            ]
        );
    }
}
