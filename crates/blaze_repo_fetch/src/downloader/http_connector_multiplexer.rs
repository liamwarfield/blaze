//! Port of `HttpConnectorMultiplexer`.

use std::sync::Arc;

use blaze_events::Event;
use blaze_events::handler::EventHandler;
use indexmap::IndexMap;

use super::checksum::Checksum;
use super::http_connector::{HeaderFunction, HttpConnector};
use super::http_stream::{HttpStream, HttpStreamFactory};
use super::http_utils;
use super::streams::Reconnector;
use crate::authandtls::{Credentials, Headers};
use crate::errors::IoException;
use crate::uri::JavaUri;

/// `REQUEST_HEADERS`: not overridable by user provided headers.
fn request_headers() -> Headers {
    let mut h = IndexMap::new();
    h.insert("Accept-Encoding".to_string(), vec!["gzip".to_string()]);
    h.insert(
        "User-Agent".to_string(),
        vec![format!("Bazel/release {}", crate::BAZEL_VERSION)],
    );
    h
}

/// Class for establishing HTTP connections.
// Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpConnectorMultiplexer.java:47 HttpConnectorMultiplexer
pub struct HttpConnectorMultiplexer {
    event_handler: Arc<dyn EventHandler>,
    connector: Arc<HttpConnector>,
    http_stream_factory: HttpStreamFactory,
}

/// `ImmutableMap.Builder.putAll(...).buildKeepingLast()`: later keys override, keeping the
/// first insertion position.
fn put_all_keeping_last(base: &mut Headers, more: &Headers) {
    for (k, v) in more {
        base.insert(k.clone(), v.clone());
    }
}

impl HttpConnectorMultiplexer {
    pub fn new(
        event_handler: Arc<dyn EventHandler>,
        connector: Arc<HttpConnector>,
        http_stream_factory: HttpStreamFactory,
    ) -> Self {
        HttpConnectorMultiplexer {
            event_handler,
            connector,
            http_stream_factory,
        }
    }

    /// Establishes a reliable HTTP connection to a URL (file, http or https).
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpConnectorMultiplexer.java:94 HttpConnectorMultiplexer.connect
    pub fn connect(
        &self,
        url: &JavaUri,
        checksum: Option<&Checksum>,
        headers: &Headers,
        credentials: Arc<dyn Credentials>,
        type_: Option<&str>,
    ) -> Result<HttpStream, IoException> {
        assert!(
            http_utils::is_url_supported_by_downloader(url),
            "IllegalArgumentException"
        );
        let mut base_headers = headers.clone();
        // REQUEST_HEADERS should not be overridable by user provided headers
        put_all_keeping_last(&mut base_headers, &request_headers());
        let header_function =
            get_header_function(base_headers, credentials, self.event_handler.clone());
        let connection = self.connector.connect(url, &header_function)?;
        let connection_url = connection.get_url().clone();
        let connector = self.connector.clone();
        let event_handler = self.event_handler.clone();
        let original = url.clone();
        let reconnector: Reconnector = Arc::new(move |cause: &IoException, extra_headers| {
            event_handler.handle(&Event::progress(format!(
                "Lost connection for {} due to {}",
                original,
                cause.to_java_string()
            )));
            let hf = header_function.clone();
            let extra: Headers = extra_headers.into_iter().collect();
            let combined: HeaderFunction = Arc::new(move |new_url: &JavaUri| {
                let mut h = hf(new_url);
                for (k, v) in &extra {
                    // ImmutableMap.Builder.buildOrThrow: duplicate keys are an error.
                    if h.contains_key(k) {
                        panic!(
                            "IllegalArgumentException: Multiple entries with same key: {}",
                            k
                        );
                    }
                    h.insert(k.clone(), v.clone());
                }
                h
            });
            connector.connect(&connection_url, &combined)
        });
        self.http_stream_factory
            .create(connection, url, checksum, reconnector, type_)
    }
}

/// Headers for a URL: the base headers plus the credentials' request metadata.
// Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpConnectorMultiplexer.java:138 HttpConnectorMultiplexer.getHeaderFunction
pub fn get_header_function(
    base_headers: Headers,
    credentials: Arc<dyn Credentials>,
    event_handler: Arc<dyn EventHandler>,
) -> HeaderFunction {
    Arc::new(move |url: &JavaUri| {
        let mut headers = base_headers.clone();
        match credentials.get_request_metadata(url) {
            Ok(m) => put_all_keeping_last(&mut headers, &m),
            Err(e) => {
                // If fetching credentials fails for any reason, still try to do the connection,
                // not adding authentication information as we cannot look it up.
                event_handler.handle(&Event::warn(format!(
                    "Error retrieving auth headers, continuing without: {}",
                    e.get_message()
                )));
            }
        }
        headers
    })
}
