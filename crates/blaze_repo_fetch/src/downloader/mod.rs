//! Port of `com.google.devtools.build.lib.bazel.repository.downloader`.

pub mod checksum;
pub mod download_manager;
pub mod download_progress_event;
pub mod http_connector;
pub mod http_connector_multiplexer;
pub mod http_downloader;
pub mod http_stream;
pub mod http_utils;
pub mod jdk_http_auth;
pub mod proxy_helper;
pub mod socket_transport;
pub mod streams;
pub mod url_rewriter;

pub use checksum::{Checksum, InvalidChecksumException};
pub use download_manager::DownloadManager;
pub use download_progress_event::DownloadProgressEvent;
pub use http_downloader::{DelegatingDownloader, Downloader, HttpDownloader};
pub use url_rewriter::{RewrittenUrl, UrlRewriter, UrlRewriterParseException};
