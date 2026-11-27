//! Port of `HttpStream`: input stream that validates the checksum and resumes downloads on
//! error.

use std::sync::Arc;

use blaze_events::handler::ExtendedEventHandler;
use blaze_events::ui::Clock;

use super::checksum::Checksum;
use super::http_connector::UrlConnection;
use super::http_utils;
use super::streams::{
    CheckContentLengthInputStream, GzipInputStream, HashInputStream, InputStream,
    PrefixedInputStream, ProgressInputStream, Reconnector, RetryingInputStream,
};
use crate::errors::IoException;
use crate::uri::JavaUri;

pub const PRECHECK_BYTES: usize = 32 * 1024;
const GZIPPED_EXTENSIONS: [&str; 2] = ["gz", "tgz"];
const GZIP_CONTENT_ENCODING: [&str; 2] = ["gzip", "x-gzip"];

/// An open download: the payload stream and the final (redirected) URL.
// Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpStream.java:44 HttpStream
pub struct HttpStream {
    stream: Box<dyn InputStream>,
    url: JavaUri,
}

impl HttpStream {
    /// Returns final redirected URI.
    pub fn get_url(&self) -> &JavaUri {
        &self.url
    }
}

impl InputStream for HttpStream {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, IoException> {
        self.stream.read(buf)
    }

    fn close(&mut self) -> Result<(), IoException> {
        self.stream.close()
    }
}

/// Factory for [`HttpStream`].
// Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpStream.java:55 HttpStream.Factory
#[derive(Clone)]
pub struct HttpStreamFactory {
    pub clock: Arc<dyn Clock>,
    pub event_handler: Arc<dyn ExtendedEventHandler>,
}

/// Checks if the given type (e.g. `tar.gz`) is GZIP.
fn type_is_gzip(type_: Option<&str>) -> bool {
    match type_ {
        Some(t) => {
            let t = if t.contains('.') {
                // We only want to look at the last extension.
                http_utils::get_extension(t)
            } else {
                t.to_string()
            };
            GZIPPED_EXTENSIONS.contains(&t.as_str())
        }
        None => false,
    }
}

impl HttpStreamFactory {
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpStream.java:73 HttpStream.Factory.create
    pub fn create(
        &self,
        mut connection: UrlConnection,
        original_url: &JavaUri,
        checksum: Option<&Checksum>,
        reconnector: Reconnector,
        type_: Option<&str>,
    ) -> Result<HttpStream, IoException> {
        let mut stream: Box<dyn InputStream> = Box::new(
            super::streams::InterruptibleInputStream::new(connection.get_input_stream()?),
        );
        let connection_url = connection.get_url().clone();
        let result = (|| -> Result<Box<dyn InputStream>, (IoException, Box<dyn InputStream>)> {
            // PORT: the Content-Length header is read before the RetryingInputStream is built
            // (Java reads it after; reading a header has no effect) because the retrying stream
            // must know that a short body is reported by the CheckContentLengthInputStream and
            // is not a read error to resume from. See RetryingInputStream.with_fixed_length_body.
            let content_length: Option<u64> = connection
                .get_header_field("Content-Length")
                .and_then(|content_length| content_length.parse::<u64>().ok());
            // If server supports range requests, we can retry on read errors.
            if connection
                .get_header_field("Accept-Ranges")
                .unwrap_or_default()
                .split(',')
                .any(|s| s.trim() == "bytes")
            {
                let retrying = RetryingInputStream::new(stream, reconnector);
                stream = if content_length.is_some() {
                    Box::new(retrying.with_fixed_length_body())
                } else {
                    Box::new(retrying)
                };
            }
            let mut total_bytes = None;
            if let Some(n) = content_length {
                total_bytes = Some(n as i64);
                stream = Box::new(CheckContentLengthInputStream::new(stream, n as i64));
            }
            stream = Box::new(ProgressInputStream::create(
                self.clock.clone(),
                self.event_handler.clone(),
                stream,
                connection_url.clone(),
                original_url.clone(),
                total_bytes,
            ));
            // Determine if we need to transparently gunzip. Some web servers send
            // Content-Encoding: gzip even when we didn't request it if the file is a .gz file;
            // if the file is already compressed we should not decompress it.
            let connection_path = connection_url.get_path().unwrap_or_default();
            let original_path = original_url.get_path().unwrap_or_default();
            if GZIP_CONTENT_ENCODING.contains(
                &connection
                    .get_content_encoding()
                    .unwrap_or_default()
                    .as_str(),
            ) && !GZIPPED_EXTENSIONS
                .contains(&http_utils::get_extension(&connection_path).as_str())
                && !GZIPPED_EXTENSIONS.contains(&http_utils::get_extension(&original_path).as_str())
                && !type_is_gzip(type_)
            {
                match GzipInputStream::new(stream) {
                    Ok(g) => stream = Box::new(g),
                    Err((e, inner)) => return Err((e, inner)),
                }
            }
            if let Some(checksum) = checksum {
                stream = Box::new(HashInputStream::new(stream, checksum.clone()));
                let mut buffer = vec![0u8; PRECHECK_BYTES];
                let mut read = 0;
                while read < PRECHECK_BYTES {
                    match stream.read(&mut buffer[read..]) {
                        Ok(0) => break,
                        Ok(n) => read += n,
                        Err(e) => return Err((e, stream)),
                    }
                }
                if read < PRECHECK_BYTES {
                    if let Err(e) = stream.close() {
                        return Err((e, stream));
                    }
                    buffer.truncate(read);
                    stream = Box::new(PrefixedInputStream::new(buffer, None));
                } else {
                    stream = Box::new(PrefixedInputStream::new(buffer, Some(stream)));
                }
            }
            Ok(stream)
        })();
        match result {
            Ok(stream) => Ok(HttpStream {
                stream,
                url: connection_url,
            }),
            Err((mut e, mut stream)) => {
                if let Err(e2) = stream.close() {
                    e.add_suppressed(e2);
                }
                Err(e)
            }
        }
    }
}
