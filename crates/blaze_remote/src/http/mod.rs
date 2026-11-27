//! Port of `lib/remote/http/`: the HTTP/1.1 remote cache client.
//!
//! PORT: Java builds the client on Netty (an event loop, a channel pool and a pipeline of
//! handlers per request). Blaze reads the responses with the downloader's HTTP stack
//! (`blaze_repo_fetch::downloader::socket_transport`: the response head and the body framing)
//! over blocking connections of its own pool; the handlers' behaviour (requests, status
//! handling, keep-alive, timeouts, content decoding) is ported in [`http_cache_client`], the
//! Netty classes themselves (`AbstractHttpHandler`, `HttpDownloadHandler`, `HttpUploadHandler`,
//! `IdleTimeoutHandler`, `DownloadCommand`, `UploadCommand`) are not.

pub mod http_cache_client;
mod snappy;

use std::error::Error;
use std::fmt;
use std::io;

/// An exception that propagates the http status.
///
/// PORT: Java keeps the whole Netty `HttpResponse`; the status and the header fields are what
/// its users read (`HTTP_RESULT_CLASSIFIER`, `authTokenExpired`, `cacheMiss`).
// Java: com/google/devtools/build/lib/remote/http/HttpException.java:22 HttpException
#[derive(Debug, Clone)]
pub struct HttpException {
    /// `response().status().code()`.
    pub status: u16,
    /// `response().status().reasonPhrase()`.
    pub reason_phrase: String,
    /// `response().headers()`: lower-cased names, in order.
    pub headers: Vec<(String, String)>,
    message: String,
}

impl HttpException {
    // Java: com/google/devtools/build/lib/remote/http/HttpException.java:25 HttpException.<init>
    pub fn new(
        status: u16,
        reason_phrase: String,
        headers: Vec<(String, String)>,
        message: String,
    ) -> HttpException {
        HttpException {
            status,
            reason_phrase,
            headers,
            message,
        }
    }

    /// All values of a header field (lower-case name), in order.
    pub fn header_values(&self, name: &str) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
            .collect()
    }
}

impl fmt::Display for HttpException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for HttpException {}

/// `DownloadTimeoutException`.
// Java: com/google/devtools/build/lib/remote/http/DownloadTimeoutException.java:20 DownloadTimeoutException
#[derive(Debug, Clone)]
pub struct DownloadTimeoutException {
    message: String,
}

impl DownloadTimeoutException {
    // Java: com/google/devtools/build/lib/remote/http/DownloadTimeoutException.java:22 DownloadTimeoutException.<init>
    pub fn new(url: &str, bytes_received: i64, content_length: i64) -> DownloadTimeoutException {
        DownloadTimeoutException {
            message: Self::build_message(url, bytes_received, content_length),
        }
    }

    // Java: com/google/devtools/build/lib/remote/http/DownloadTimeoutException.java:26 DownloadTimeoutException.buildMessage
    fn build_message(url: &str, bytes_received: i64, content_length: i64) -> String {
        if content_length < 0 {
            format!("Download of '{url}' timed out. Received {bytes_received} bytes.")
        } else {
            format!(
                "Download of '{url}' timed out. Received {bytes_received} of {content_length} bytes."
            )
        }
    }
}

impl fmt::Display for DownloadTimeoutException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for DownloadTimeoutException {}

/// `UploadTimeoutException`.
// Java: com/google/devtools/build/lib/remote/http/UploadTimeoutException.java:18 UploadTimeoutException
#[derive(Debug, Clone)]
pub struct UploadTimeoutException {
    message: String,
}

impl UploadTimeoutException {
    // Java: com/google/devtools/build/lib/remote/http/UploadTimeoutException.java:20 UploadTimeoutException.<init>
    pub fn new(url: &str, content_length: i64) -> UploadTimeoutException {
        UploadTimeoutException {
            // Java: UploadTimeoutException.buildMessage
            message: format!("Upload of '{url}' timed out. Sent {content_length} bytes."),
        }
    }
}

impl fmt::Display for UploadTimeoutException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for UploadTimeoutException {}

/// `java.nio.channels.ClosedChannelException`: the connection closed before the response was
/// complete. Its message is null (displayed empty; `Utils.grpcAwareErrorMessage` then prints the
/// class's simple name).
#[derive(Debug, Clone, Default)]
pub struct ClosedChannelException;

impl fmt::Display for ClosedChannelException {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Ok(())
    }
}

impl Error for ClosedChannelException {}

impl From<ClosedChannelException> for io::Error {
    fn from(e: ClosedChannelException) -> io::Error {
        io::Error::new(io::ErrorKind::ConnectionAborted, e)
    }
}

impl From<HttpException> for io::Error {
    fn from(e: HttpException) -> io::Error {
        io::Error::other(e)
    }
}

impl From<DownloadTimeoutException> for io::Error {
    fn from(e: DownloadTimeoutException) -> io::Error {
        io::Error::new(io::ErrorKind::TimedOut, e)
    }
}

impl From<UploadTimeoutException> for io::Error {
    fn from(e: UploadTimeoutException) -> io::Error {
        io::Error::new(io::ErrorKind::TimedOut, e)
    }
}
