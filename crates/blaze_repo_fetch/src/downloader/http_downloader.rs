//! Ports of `Downloader`, `DelegatingDownloader` and `HttpDownloader`.

use std::collections::HashMap;
use std::io::Write as _;
use std::sync::Arc;
use std::time::Duration;

use blaze_events::Event;
use blaze_events::handler::ExtendedEventHandler;
use blaze_events::ui::{Clock, JavaClock};
use parking_lot::{Condvar, Mutex, RwLock};

use super::checksum::Checksum;
use super::download_progress_event::FetchEvent;
use super::http_connector::{
    HttpConnector, HttpTransport, JavaSleeper, Sleeper, default_transport,
};
use super::http_connector_multiplexer::HttpConnectorMultiplexer;
use super::http_stream::{HttpStream, HttpStreamFactory};
use super::proxy_helper::ProxyHelper;
use super::streams::{InputStream, transfer_to};
use crate::authandtls::{Credentials, Headers};
use crate::errors::{IoException, IoExceptionKind};
use crate::uri::{JavaUri, uri_list_to_string};
use blaze_vfs::Path;

/// Interface for implementing the download of a file.
// Java: com/google/devtools/build/lib/bazel/repository/downloader/Downloader.java:28 Downloader
pub trait Downloader: Send + Sync {
    /// Downloads a file and writes it to `output`. On failure the output file is in an
    /// undefined state.
    fn download(
        &self,
        urls: &[JavaUri],
        headers: &Headers,
        credentials: Arc<dyn Credentials>,
        checksum: Option<&Checksum>,
        canonical_id: &str,
        output: &Path,
        event_handler: Arc<dyn ExtendedEventHandler>,
        client_env: &HashMap<String, String>,
        type_: Option<&str>,
        context: &str,
    ) -> Result<(), IoException>;
}

/// A [`Downloader`] that delegates to another one (mutable dependency injection).
// Java: com/google/devtools/build/lib/bazel/repository/downloader/DelegatingDownloader.java:29 DelegatingDownloader
pub struct DelegatingDownloader {
    default_delegate: Arc<dyn Downloader>,
    delegate: RwLock<Option<Arc<dyn Downloader>>>,
}

impl DelegatingDownloader {
    pub fn new(default_delegate: Arc<dyn Downloader>) -> Self {
        DelegatingDownloader {
            default_delegate,
            delegate: RwLock::new(None),
        }
    }

    /// Sets the delegate; `None` restores the default delegate.
    pub fn set_delegate(&self, delegate: Option<Arc<dyn Downloader>>) {
        *self.delegate.write() = delegate;
    }
}

impl Downloader for DelegatingDownloader {
    fn download(
        &self,
        urls: &[JavaUri],
        headers: &Headers,
        credentials: Arc<dyn Credentials>,
        checksum: Option<&Checksum>,
        canonical_id: &str,
        output: &Path,
        event_handler: Arc<dyn ExtendedEventHandler>,
        client_env: &HashMap<String, String>,
        type_: Option<&str>,
        context: &str,
    ) -> Result<(), IoException> {
        let downloader = self
            .delegate
            .read()
            .clone()
            .unwrap_or_else(|| self.default_delegate.clone());
        downloader.download(
            urls,
            headers,
            credentials,
            checksum,
            canonical_id,
            output,
            event_handler,
            client_env,
            type_,
            context,
        )
    }
}

/// A fair counting semaphore (`java.util.concurrent.Semaphore`).
struct Semaphore {
    permits: Mutex<i32>,
    cv: Condvar,
}

impl Semaphore {
    fn acquire(&self) {
        let mut p = self.permits.lock();
        while *p <= 0 {
            self.cv.wait(&mut p);
        }
        *p -= 1;
    }

    fn release(&self) {
        *self.permits.lock() += 1;
        self.cv.notify_one();
    }
}

/// HTTP implementation of [`Downloader`].
// Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpDownloader.java:49 HttpDownloader
pub struct HttpDownloader {
    semaphore: Semaphore,
    timeout_scaling: f32,
    max_attempts: i32,
    max_retry_timeout: Duration,
    clock: Arc<dyn Clock>,
    sleeper: Arc<dyn Sleeper>,
    transport: Arc<dyn HttpTransport>,
}

impl HttpDownloader {
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpDownloader.java:58 HttpDownloader.<init>
    pub fn new(
        max_attempts: i32,
        max_retry_timeout: Duration,
        max_parallel_downloads: i32,
        timeout_scaling: f32,
    ) -> Self {
        HttpDownloader {
            semaphore: Semaphore {
                permits: Mutex::new(max_parallel_downloads),
                cv: Condvar::new(),
            },
            timeout_scaling,
            max_attempts,
            max_retry_timeout,
            clock: Arc::new(JavaClock),
            sleeper: Arc::new(JavaSleeper),
            transport: default_transport(),
        }
    }

    /// `new HttpDownloader()`: `(0, Duration.ZERO, 8, 1.0f)`.
    pub fn default_downloader() -> Self {
        Self::new(0, Duration::ZERO, 8, 1.0)
    }

    /// Test hook: replaces the clock, sleeper and transport.
    pub fn with_test_hooks(
        mut self,
        clock: Arc<dyn Clock>,
        sleeper: Arc<dyn Sleeper>,
        transport: Option<Arc<dyn HttpTransport>>,
    ) -> Self {
        self.clock = clock;
        self.sleeper = sleeper;
        if let Some(t) = transport {
            self.transport = t;
        }
        self
    }

    /// Downloads the contents of the first of the given URLs that succeeds into memory. If all
    /// fail, the last failure is returned with the earlier ones suppressed.
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpDownloader.java:114 HttpDownloader.downloadAndRead
    pub fn download_and_read(
        &self,
        urls: &[JavaUri],
        credentials: Arc<dyn Credentials>,
        checksum: Option<&Checksum>,
        event_handler: Arc<dyn ExtendedEventHandler>,
        client_env: &HashMap<String, String>,
    ) -> Result<Vec<u8>, IoException> {
        assert!(
            !urls.is_empty(),
            "IllegalArgumentException: Cannot download from an empty list of URLs"
        );
        self.download_from_urls(
            urls,
            &Headers::new(),
            credentials,
            checksum,
            None,
            event_handler,
            client_env,
            false,
            &mut |payload| {
                let mut out = Vec::new();
                transfer_to(payload, &mut out)?;
                Ok(out)
            },
            &|io_exceptions| last_download_exception(io_exceptions),
        )
    }

    // Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpDownloader.java:156 HttpDownloader.downloadFromUrls
    fn download_from_urls<T>(
        &self,
        urls: &[JavaUri],
        headers: &Headers,
        credentials: Arc<dyn Credentials>,
        checksum: Option<&Checksum>,
        type_: Option<&str>,
        event_handler: Arc<dyn ExtendedEventHandler>,
        client_env: &HashMap<String, String>,
        report_progress_events: bool,
        processor: &mut dyn FnMut(&mut HttpStream) -> Result<T, IoException>,
        on_all_urls_failed: &dyn Fn(Vec<IoException>) -> IoException,
    ) -> Result<T, IoException> {
        let multiplexer = self.set_up_connector_multiplexer(event_handler.clone(), client_env);
        let mut io_exceptions: Vec<IoException> = Vec::new();
        for url in urls {
            self.semaphore.acquire();
            let mut success = false;
            let result =
                match multiplexer.connect(url, checksum, headers, credentials.clone(), type_) {
                    Ok(mut payload) => {
                        let r = processor(&mut payload);
                        if r.is_ok() {
                            success = true;
                        }
                        let closed = payload.close();
                        match (r, closed) {
                            (Ok(v), Ok(())) => Ok(v),
                            (Ok(_), Err(e)) => Err(e),
                            (Err(mut e), Err(e2)) => {
                                e.add_suppressed(e2);
                                Err(e)
                            }
                            (Err(e), Ok(())) => Err(e),
                        }
                    }
                    Err(e) => Err(e),
                };
            self.semaphore.release();
            if report_progress_events {
                event_handler.post(Arc::new(FetchEvent {
                    url: url.to_string(),
                    downloader: "HTTP",
                    success,
                }));
            }
            match result {
                Ok(v) => return Ok(v),
                Err(e) if e.kind == IoExceptionKind::SocketTimeoutException => {
                    // Not an external interruption, but simply a failed download due to some
                    // server timing out. So treat it as an ordinary IOException.
                    record_failure(
                        &mut io_exceptions,
                        IoException::wrapping(&e),
                        url,
                        &*event_handler,
                        report_progress_events,
                    );
                }
                Err(e) if e.kind == IoExceptionKind::InterruptedIOException => return Err(e),
                Err(e) => record_failure(
                    &mut io_exceptions,
                    e,
                    url,
                    &*event_handler,
                    report_progress_events,
                ),
            }
        }
        Err(on_all_urls_failed(io_exceptions))
    }

    // Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpDownloader.java:269 HttpDownloader.setUpConnectorMultiplexer
    fn set_up_connector_multiplexer(
        &self,
        event_handler: Arc<dyn ExtendedEventHandler>,
        client_env: &HashMap<String, String>,
    ) -> HttpConnectorMultiplexer {
        let proxy_helper = ProxyHelper::new(client_env.clone());
        let plain: Arc<dyn blaze_events::handler::EventHandler> =
            Arc::new(ExtendedAsPlain(event_handler.clone()));
        let connector = HttpConnector::new(
            plain.clone(),
            proxy_helper,
            self.sleeper.clone(),
            self.timeout_scaling,
            self.max_attempts,
            self.max_retry_timeout,
            self.transport.clone(),
        );
        let http_stream_factory = HttpStreamFactory {
            clock: self.clock.clone(),
            event_handler,
        };
        HttpConnectorMultiplexer::new(plain, Arc::new(connector), http_stream_factory)
    }
}

/// Adapts an `ExtendedEventHandler` to the plain `EventHandler` the connector takes.
struct ExtendedAsPlain(Arc<dyn ExtendedEventHandler>);

impl blaze_events::handler::EventHandler for ExtendedAsPlain {
    fn handle(&self, event: &Event) {
        self.0.handle(event)
    }
}

// Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpDownloader.java:211 HttpDownloader.recordFailure
fn record_failure(
    io_exceptions: &mut Vec<IoException>,
    failure: IoException,
    url: &JavaUri,
    event_handler: &dyn ExtendedEventHandler,
    report_progress_events: bool,
) {
    if report_progress_events {
        event_handler.handle(&Event::warn(format!(
            "Download from {} failed: {} {}",
            url,
            failure.class_string(),
            failure.get_message()
        )));
    }
    io_exceptions.push(failure);
}

// Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpDownloader.java:240 HttpDownloader.aggregatedDownloadException
fn aggregated_download_exception(
    urls: &[JavaUri],
    destination: &Path,
    io_exceptions: Vec<IoException>,
) -> IoException {
    let mut exception = IoException::new(format!(
        "Error downloading {} to {}{}",
        uri_list_to_string(urls),
        destination,
        match io_exceptions.last() {
            None => String::new(),
            Some(e) => format!(": {}", e.get_message()),
        }
    ));
    for cause in io_exceptions {
        exception.add_suppressed(cause);
    }
    exception
}

// Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpDownloader.java:261 HttpDownloader.lastDownloadException
fn last_download_exception(mut io_exceptions: Vec<IoException>) -> IoException {
    let mut last = io_exceptions.pop().expect("non-empty");
    for e in io_exceptions {
        last.add_suppressed(e);
    }
    last
}

impl Downloader for HttpDownloader {
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpDownloader.java:73 HttpDownloader.download
    fn download(
        &self,
        urls: &[JavaUri],
        headers: &Headers,
        credentials: Arc<dyn Credentials>,
        checksum: Option<&Checksum>,
        _canonical_id: &str,
        destination: &Path,
        event_handler: Arc<dyn ExtendedEventHandler>,
        client_env: &HashMap<String, String>,
        type_: Option<&str>,
        _context: &str,
    ) -> Result<(), IoException> {
        // Stream the payload straight to the destination file and report progress to the CLI.
        // On total failure, wrap every per-URL failure in a single new IOException naming the
        // URLs and the destination.
        self.download_from_urls(
            urls,
            headers,
            credentials,
            checksum,
            type_,
            event_handler,
            client_env,
            true,
            &mut |payload| {
                let mut out = destination.get_output_stream()?;
                transfer_to(payload, &mut out)?;
                out.flush()?;
                Ok(())
            },
            &|io_exceptions| aggregated_download_exception(urls, destination, io_exceptions),
        )
    }
}
