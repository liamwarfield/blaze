//! A [`RegistryFetcher`](blaze_bzlmod_model::RegistryFetcher) over the downloader and the
//! repository cache (`DownloadManager.downloadAndReadOneUrlForBzlmod`). OWNER: agent C.
//!
//! - [`LocalDownloadManagerFetcher`]: a local port of exactly the slice of `DownloadManager`,
//!   `HttpDownloader`, `HttpConnector(Multiplexer)`, `HttpStream` and `DownloadCache` that Bazel
//!   uses for registry files. It shares the repository cache with real Bazel.
//! - `DownloadManagerRegistryFetcher` (feature `repo-fetch`): the same seam over
//!   `blaze_repo_fetch`'s full `DownloadManager`.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use blaze_bzlmod_model::registry::{RegistryException, RegistryFetcher};
use blaze_bzlmod_model::{Checksum, JavaUri, KeyType};
use blaze_events::{ExtendedEventHandler, Postable};
use indexmap::IndexMap;

pub mod download_cache;
pub mod http;

pub use download_cache::LocalDownloadCache;
pub use http::LocalHttpDownloader;

/// The Java class of an [`IoException`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IoExceptionKind {
    /// `java.io.IOException`
    IOException,
    /// `java.io.FileNotFoundException`
    FileNotFoundException,
    /// `java.io.EOFException`
    EOFException,
    /// `java.io.InterruptedIOException`
    InterruptedIOException,
    /// `java.net.SocketTimeoutException` (a subclass of `InterruptedIOException`)
    SocketTimeoutException,
    /// `java.net.SocketException`
    SocketException,
    /// `java.net.ConnectException` (a subclass of `SocketException`)
    ConnectException,
    /// `java.net.UnknownHostException`
    UnknownHostException,
    /// `javax.net.ssl.SSLException`
    SSLException,
    /// `java.util.zip.ZipException`
    ZipException,
    /// `UnrecoverableHttpException`
    UnrecoverableHttpException,
    /// `ContentLengthMismatchException`
    ContentLengthMismatchException,
    /// `IllegalArgumentException` raised by `HttpURLConnection` / `URI.create`; HttpConnector
    /// turns it into an `UnrecoverableHttpException` before it leaves `connect`.
    IllegalArgumentException,
}

impl IoExceptionKind {
    /// `Class.getName()`.
    pub fn java_class_name(self) -> &'static str {
        match self {
            IoExceptionKind::IOException => "java.io.IOException",
            IoExceptionKind::FileNotFoundException => "java.io.FileNotFoundException",
            IoExceptionKind::EOFException => "java.io.EOFException",
            IoExceptionKind::InterruptedIOException => "java.io.InterruptedIOException",
            IoExceptionKind::SocketTimeoutException => "java.net.SocketTimeoutException",
            IoExceptionKind::SocketException => "java.net.SocketException",
            IoExceptionKind::ConnectException => "java.net.ConnectException",
            IoExceptionKind::UnknownHostException => "java.net.UnknownHostException",
            IoExceptionKind::SSLException => "javax.net.ssl.SSLException",
            IoExceptionKind::ZipException => "java.util.zip.ZipException",
            IoExceptionKind::UnrecoverableHttpException => {
                "com.google.devtools.build.lib.bazel.repository.downloader.UnrecoverableHttpException"
            }
            IoExceptionKind::ContentLengthMismatchException => {
                "com.google.devtools.build.lib.bazel.repository.downloader.ContentLengthMismatchException"
            }
            IoExceptionKind::IllegalArgumentException => "java.lang.IllegalArgumentException",
        }
    }
}

/// A Java `IOException` (or subclass) raised by the download path: class, message and
/// suppressed exceptions.
// PORT: Java distinguishes exceptions by type; one struct with a kind keeps both the type tests
// (`instanceof`) and the printed class names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IoException {
    pub kind: IoExceptionKind,
    /// `Throwable.getMessage()`; `None` is Java `null`.
    pub message: Option<String>,
    /// `Throwable.getSuppressed()`.
    pub suppressed: Vec<IoException>,
}

impl IoException {
    /// `new IOException(message)`.
    pub fn new(message: impl Into<String>) -> IoException {
        IoException::of(IoExceptionKind::IOException, message)
    }

    pub fn of(kind: IoExceptionKind, message: impl Into<String>) -> IoException {
        IoException {
            kind,
            message: Some(message.into()),
            suppressed: Vec::new(),
        }
    }

    pub fn without_message(kind: IoExceptionKind) -> IoException {
        IoException {
            kind,
            message: None,
            suppressed: Vec::new(),
        }
    }

    /// `new IOException(cause)`: the message is `cause.toString()`.
    pub fn wrapping(cause: &IoException) -> IoException {
        IoException::new(cause.to_java_string())
    }

    /// An I/O error on `path`, worded as Java's file system does (`<path> (<reason>)`).
    pub fn from_io(path: &std::path::Path, e: &std::io::Error) -> IoException {
        let kind = if e.kind() == std::io::ErrorKind::NotFound {
            IoExceptionKind::FileNotFoundException
        } else {
            IoExceptionKind::IOException
        };
        IoException::of(kind, format!("{} ({})", path.display(), java_io_message(e)))
    }

    /// `Throwable.getMessage()`, with Java's `"null"` for a missing message.
    pub fn get_message(&self) -> String {
        self.message.clone().unwrap_or_else(|| "null".to_string())
    }

    /// `Throwable.addSuppressed`.
    pub fn add_suppressed(&mut self, e: IoException) {
        self.suppressed.push(e);
    }

    /// `Throwable.toString()`: the class name, then `": " + message` when there is one.
    pub fn to_java_string(&self) -> String {
        match &self.message {
            Some(m) => format!("{}: {}", self.kind.java_class_name(), m),
            None => self.kind.java_class_name().to_string(),
        }
    }

    /// `DownloadManager.isRetryableException`.
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/DownloadManager.java:390 DownloadManager.isRetryableException
    fn is_retryable(&self) -> bool {
        matches!(
            self.kind,
            IoExceptionKind::ContentLengthMismatchException
                | IoExceptionKind::SocketException
                | IoExceptionKind::ConnectException
                | IoExceptionKind::UnknownHostException
        )
    }
}

impl fmt::Display for IoException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.get_message())
    }
}

impl std::error::Error for IoException {}

/// The `strerror` text of an I/O error (Rust's message without the ` (os error N)` suffix).
pub fn java_io_message(e: &std::io::Error) -> String {
    let s = e.to_string();
    match s.rfind(" (os error ") {
        Some(i) if s.ends_with(')') => s[..i].to_string(),
        _ => s,
    }
}

/// Maps the download path's exception to the registry seam's error: `FileNotFoundException` is
/// [`RegistryException::NotFound`], an interrupt is [`RegistryException::Interrupted`], and any
/// other `IOException` is [`RegistryException::Io`] with its message.
pub fn to_registry_exception(e: IoException) -> RegistryException {
    match e.kind {
        IoExceptionKind::FileNotFoundException => RegistryException::NotFound(e.get_message()),
        IoExceptionKind::InterruptedIOException => RegistryException::Interrupted,
        _ => RegistryException::Io(e.get_message()),
    }
}

/// Event reporting about cache hits for download requests. Port of `DownloadCacheHitEvent`.
// Java: com/google/devtools/build/lib/bazel/repository/cache/DownloadCacheHitEvent.java:21 DownloadCacheHitEvent
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadCacheHitEvent {
    pub context: String,
    pub file_hash: String,
    pub uri: JavaUri,
}

impl Postable for DownloadCacheHitEvent {}

/// The repository cache's download (content-addressable) directory (`RepositoryCache.CAS_DIR`).
// Java: com/google/devtools/build/lib/bazel/repository/cache/RepositoryCache.java:26 RepositoryCache.CAS_DIR
pub const CAS_DIR: &str = "content_addressable";

/// Bazel's file downloader, reduced to `downloadAndReadOneUrlForBzlmod`. Port of the
/// `DownloadManager` slice used for registry files.
///
/// Configuration as `BazelRepositoryModule.beforeCommand` sets it: the repository cache path
/// (`RepositoryCache.setPath`, `content_addressable/` below it), `--experimental_repository_downloader_retries`
/// (`setRetries`), and the bzlmod `HttpDownloader` built from `--http_connector_attempts`,
/// `--http_connector_retry_max_timeout`, `--http_max_parallel_downloads` and
/// `--http_timeout_scaling`. The URL rewriter is the empty one (no `--downloader_config`), and
/// there are no credentials (no `.netrc`, no `--credential_helper`); configuring either fails
/// loudly.
// PORT: move to blaze_repo_fetch — temporary stand-in for DownloadManager while blaze_repo_fetch
// cannot build next to starlark (download_cache.rs:56)
// Java: com/google/devtools/build/lib/bazel/repository/downloader/DownloadManager.java:64 DownloadManager
pub struct LocalDownloadManagerFetcher {
    download_cache: LocalDownloadCache,
    bzlmod_http_downloader: LocalHttpDownloader,
    event_handler: Arc<dyn ExtendedEventHandler>,
    retries: i32,
}

impl LocalDownloadManagerFetcher {
    /// `repository_cache` is the `--repository_cache` directory (`None` disables the cache);
    /// `retries` is `--experimental_repository_downloader_retries` (Bazel's default is 5). The
    /// HTTP downloader starts with the command-line defaults: 8 attempts, no retry timeout cap,
    /// 8 parallel downloads, timeout scaling 1.0.
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/DownloadManager.java:89 DownloadManager.<init>
    pub fn new(
        repository_cache: Option<PathBuf>,
        event_handler: Arc<dyn ExtendedEventHandler>,
        retries: i32,
    ) -> Self {
        let mut fetcher = LocalDownloadManagerFetcher {
            download_cache: LocalDownloadCache::new(repository_cache.map(|p| p.join(CAS_DIR))),
            bzlmod_http_downloader: LocalHttpDownloader::new(8, Duration::ZERO, 8, 1.0),
            event_handler,
            retries: 0,
        };
        fetcher.set_retries(retries);
        fetcher
    }

    // Java: com/google/devtools/build/lib/bazel/repository/downloader/DownloadManager.java:112 DownloadManager.setRetries
    pub fn set_retries(&mut self, retries: i32) {
        blaze_util::check_argument!(retries >= 0, "Invalid retries");
        self.retries = retries;
    }

    /// Replaces the bzlmod HTTP downloader (`new HttpDownloader(maxAttempts, maxRetryTimeout,
    /// maxParallelDownloads, timeoutScaling)`).
    pub fn set_http_downloader(&mut self, downloader: LocalHttpDownloader) {
        self.bzlmod_http_downloader = downloader;
    }

    /// `setUrlRewriter(UrlRewriter.getDownloaderUrlRewriter(workspace, --downloader_config))`.
    /// Without a config the rewriter is empty, which leaves URLs and auth headers unchanged.
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/UrlRewriter.java:87 UrlRewriter.getDownloaderUrlRewriter
    pub fn set_downloader_config(&mut self, downloader_config: Option<&str>) {
        if let Some(config) = downloader_config.filter(|c| !c.is_empty()) {
            blaze_util::unported!("UrlRewriter.getDownloaderUrlRewriter", "{config}");
        }
    }

    /// `setNetrcCreds(UrlRewriter.newCredentialsFromNetrc(clientEnv, workingDirectory))`: finds
    /// the `.netrc` file (`$NETRC`, else `$HOME/.netrc`). An existing one is not ported yet.
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/UrlRewriter.java:313 UrlRewriter.newCredentialsFromNetrc
    pub fn set_netrc_creds_from_client_env(
        &mut self,
        client_env: &IndexMap<String, String>,
        working_directory: Option<&std::path::Path>,
    ) {
        let home_dir = client_env.get("HOME");
        let netrc_file_string = match client_env.get("NETRC") {
            Some(n) => Some(n.clone()),
            None => home_dir.map(|home| format!("{home}/.netrc")),
        };
        let Some(netrc_file_string) = netrc_file_string else {
            return;
        };
        // In case Bazel is not started from a valid workspace.
        let Some(working_directory) = working_directory else {
            return;
        };
        // An absolute path is used as it is; a relative one is resolved against the working
        // directory.
        let netrc_file = working_directory.join(&netrc_file_string);
        if netrc_file.exists() {
            blaze_util::unported!(
                "UrlRewriter.newCredentialsFromNetrc",
                "{}",
                netrc_file.display()
            );
        }
    }

    /// `setCredentialFactory(...)` with `--credential_helper` values. Without helpers the
    /// credentials are the static (empty) auth headers.
    pub fn set_credential_helpers(&mut self, credential_helpers: &[String]) {
        if !credential_helpers.is_empty() {
            blaze_util::unported!("CredentialHelperCredentials", "{credential_helpers:?}");
        }
    }

    pub fn get_download_cache(&self) -> &LocalDownloadCache {
        &self.download_cache
    }

    // Java: com/google/devtools/build/lib/bazel/repository/downloader/DownloadManager.java:372 DownloadManager.shouldRetryDownload
    fn should_retry_download(&self, e: &IoException, attempt: i32) -> bool {
        if attempt >= self.retries {
            return false;
        }

        if e.is_retryable() {
            return true;
        }

        e.suppressed.iter().any(IoException::is_retryable)
    }

    /// Downloads the contents of one URL and reads it into a byte array. Only meant for Bzlmod
    /// registry downloads, as it ignores `--repository_disable_download`.
    ///
    /// If the checksum and path to the repository cache are specified, attempt to load the file
    /// from the repository cache. If it doesn't exist, proceed to download the file and load it
    /// into the cache prior to returning the value.
    // PORT: no netrc credentials and the empty URL rewriter (see the setters), so the rewritten
    // URL list is the original URL and the auth headers are empty.
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/DownloadManager.java:414 DownloadManager.downloadAndReadOneUrlForBzlmod
    pub fn download_and_read_one_url_for_bzlmod_io(
        &self,
        original_url: &JavaUri,
        client_env: &IndexMap<String, String>,
        checksum: Option<&Checksum>,
    ) -> Result<Vec<u8>, IoException> {
        if self.download_cache.is_enabled()
            && let Some(checksum) = checksum
        {
            let cache_key = checksum.to_string();
            // Ignore error trying to get. We'll just download again.
            if let Ok(Some(content)) = self
                .download_cache
                .get_bytes(&cache_key, checksum.get_key_type())
            {
                // Cache hit!
                self.event_handler.post(Arc::new(DownloadCacheHitEvent {
                    context: "Bazel module fetching".to_string(),
                    file_hash: cache_key,
                    uri: original_url.clone(),
                }));
                return Ok(content);
            }
        }

        let rewritten_urls = vec![original_url.clone()];

        let mut attempt = 0;
        let content = loop {
            match self.bzlmod_http_downloader.download_and_read(
                &rewritten_urls,
                checksum,
                &self.event_handler,
                client_env,
            ) {
                Ok(c) => break c,
                Err(e) if e.kind == IoExceptionKind::InterruptedIOException => return Err(e),
                Err(e) => {
                    if !self.should_retry_download(&e, attempt) {
                        return Err(e);
                    }
                }
            }
            attempt += 1;
        };

        if self.download_cache.is_enabled() {
            match checksum {
                Some(c) => {
                    self.download_cache
                        .put_with_key(&c.to_string(), &content, c.get_key_type())?
                }
                None => {
                    self.download_cache.put(&content, KeyType::Sha256)?;
                }
            }
        }
        Ok(content)
    }
}

impl RegistryFetcher for LocalDownloadManagerFetcher {
    fn download_and_read_one_url_for_bzlmod(
        &self,
        url: &JavaUri,
        client_env: &IndexMap<String, String>,
        checksum: Option<&Checksum>,
    ) -> Result<Vec<u8>, RegistryException> {
        self.download_and_read_one_url_for_bzlmod_io(url, client_env, checksum)
            .map_err(to_registry_exception)
    }
}

/// The registry seam over `blaze_repo_fetch`'s `DownloadManager`. Converts this crate's
/// `JavaUri` / `Checksum` to `blaze_repo_fetch`'s by their string forms.
#[cfg(feature = "repo-fetch")]
pub struct DownloadManagerRegistryFetcher {
    download_manager: blaze_repo_fetch::downloader::download_manager::DownloadManager,
}

#[cfg(feature = "repo-fetch")]
impl DownloadManagerRegistryFetcher {
    pub fn new(
        download_manager: blaze_repo_fetch::downloader::download_manager::DownloadManager,
    ) -> Self {
        DownloadManagerRegistryFetcher { download_manager }
    }

    /// A download manager as `BazelRepositoryModule.beforeCommand` builds it: the repository
    /// cache at `repository_cache` (`None` disables it), the given HTTP downloader for both the
    /// delegating downloader and the bzlmod downloader, and `retries`.
    pub fn create(
        repository_cache: Option<&std::path::Path>,
        http_downloader: blaze_repo_fetch::downloader::HttpDownloader,
        event_handler: Arc<dyn ExtendedEventHandler>,
        retries: i32,
    ) -> Self {
        use blaze_repo_fetch::cache::RepositoryCache;
        use blaze_repo_fetch::downloader::{DelegatingDownloader, DownloadManager, Downloader};
        use blaze_vfs::{DigestHashFunction, Path, UnixFileSystem};

        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        let mut repository_cache_value = RepositoryCache::new();
        repository_cache_value
            .set_path(repository_cache.map(|p| Path::create_from_str(&p.to_string_lossy(), &fs)));
        let http_downloader = Arc::new(http_downloader);
        let delegating: Arc<dyn Downloader> = Arc::new(DelegatingDownloader::new(
            http_downloader.clone() as Arc<dyn Downloader>,
        ));
        let mut download_manager = DownloadManager::new(
            repository_cache_value.get_download_cache().clone(),
            delegating,
            http_downloader,
            event_handler,
        );
        download_manager.set_retries(retries);
        DownloadManagerRegistryFetcher { download_manager }
    }

    pub fn get_download_manager(
        &self,
    ) -> &blaze_repo_fetch::downloader::download_manager::DownloadManager {
        &self.download_manager
    }
}

#[cfg(feature = "repo-fetch")]
fn to_repo_fetch_checksum(checksum: &Checksum) -> blaze_repo_fetch::downloader::Checksum {
    use blaze_repo_fetch::cache::KeyType as RfKeyType;
    use blaze_repo_fetch::downloader::Checksum as RfChecksum;
    // A checksum built from an SRI string prints other hashes as SRI; keep that format so error
    // messages read the same.
    let same_format = checksum.emit_other_hash_in_same_format(checksum.get_hash_code());
    let converted = if same_format.contains('-') {
        RfChecksum::from_subresource_integrity(&checksum.to_subresource_integrity())
    } else {
        let key_type = match checksum.get_key_type() {
            KeyType::Sha1 => RfKeyType::SHA1,
            KeyType::Sha256 => RfKeyType::SHA256,
            KeyType::Sha384 => RfKeyType::SHA384,
            KeyType::Sha512 => RfKeyType::SHA512,
            KeyType::Blake3 => RfKeyType::BLAKE3,
        };
        RfChecksum::from_string(key_type, &checksum.to_string())
    };
    converted.expect("a valid checksum stays valid across the conversion")
}

#[cfg(feature = "repo-fetch")]
fn from_repo_fetch_exception(e: blaze_repo_fetch::errors::IoException) -> RegistryException {
    use blaze_repo_fetch::errors::IoExceptionKind as RfKind;
    match e.kind {
        RfKind::FileNotFoundException => RegistryException::NotFound(e.get_message()),
        // DownloadManager turns an InterruptedIOException into an InterruptedException.
        RfKind::InterruptedIOException => RegistryException::Interrupted,
        _ => RegistryException::Io(e.get_message()),
    }
}

#[cfg(feature = "repo-fetch")]
impl RegistryFetcher for DownloadManagerRegistryFetcher {
    fn download_and_read_one_url_for_bzlmod(
        &self,
        url: &JavaUri,
        client_env: &IndexMap<String, String>,
        checksum: Option<&Checksum>,
    ) -> Result<Vec<u8>, RegistryException> {
        let rf_url = blaze_repo_fetch::uri::JavaUri::create(&url.to_string())
            .map_err(|e| RegistryException::Io(e.to_string()))?;
        let rf_checksum = checksum.map(to_repo_fetch_checksum);
        let env: std::collections::HashMap<String, String> = client_env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        self.download_manager
            .download_and_read_one_url_for_bzlmod(&rf_url, &env, rf_checksum.as_ref())
            .map_err(from_repo_fetch_exception)
    }
}
