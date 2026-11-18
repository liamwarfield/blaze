//! Port of `DownloadManager`: repository cache and `--distdir` lookup, URL rewriting, retries,
//! then the [`Downloader`].

use std::collections::HashMap;
use std::sync::Arc;
use std::thread::JoinHandle;

use blaze_events::Event;
use blaze_events::handler::{ExtendedEventHandler, FetchProgress, Postable};
use blaze_vfs::{Path, PathFragment};
use indexmap::IndexSet;

use super::checksum::Checksum;
use super::http_downloader::{Downloader, HttpDownloader};
use super::url_rewriter::UrlRewriter;
use crate::authandtls::{Credentials, Headers, StaticCredentials};
use crate::cache::DownloadCacheHitEvent;
use crate::cache::download_cache::{self, DownloadCache, KeyType};
use crate::errors::{IoException, IoExceptionKind};
use crate::fs_utils;
use crate::uri::{JavaUri, uri_list_to_string};

/// Creates `Credentials` from a map of per-URI authentication headers.
pub type CredentialFactory =
    Arc<dyn Fn(&HashMap<JavaUri, Headers>) -> Arc<dyn Credentials> + Send + Sync>;

/// Progress of a cache or distdir lookup.
// Java: com/google/devtools/build/lib/bazel/repository/downloader/DownloadManager.java:568 DownloadManager.CacheProgress
#[derive(Clone, Debug)]
pub struct CacheProgress {
    original_url: String,
    progress: String,
    is_finished: bool,
}

impl Postable for CacheProgress {}

impl FetchProgress for CacheProgress {
    fn get_resource_identifier(&self) -> String {
        self.original_url.clone()
    }

    fn get_progress(&self) -> String {
        self.progress.clone()
    }

    fn is_finished(&self) -> bool {
        self.is_finished
    }
}

impl CacheProgress {
    fn started(original_url: &str, progress: String) -> Arc<dyn Postable> {
        Arc::new(CacheProgress {
            original_url: original_url.to_string(),
            progress,
            is_finished: false,
        })
    }

    fn finished(original_url: &str) -> Arc<dyn Postable> {
        Arc::new(CacheProgress {
            original_url: original_url.to_string(),
            progress: String::new(),
            is_finished: true,
        })
    }
}

/// Bazel file downloader.
// Java: com/google/devtools/build/lib/bazel/repository/downloader/DownloadManager.java:61 DownloadManager
pub struct DownloadManager {
    download_cache: DownloadCache,
    distdir: Vec<Path>,
    rewriter: Option<UrlRewriter>,
    downloader: Arc<dyn Downloader>,
    bzlmod_http_downloader: Arc<HttpDownloader>,
    event_handler: Arc<dyn ExtendedEventHandler>,
    disable_download: bool,
    retries: i32,
    netrc_creds: Option<Arc<dyn Credentials>>,
    credential_factory: CredentialFactory,
}

/// A download running on its own thread (`Future<Path>`).
pub struct PendingDownload(JoinHandle<Result<Path, IoException>>);

impl PendingDownload {
    /// Whether the download has completed, successfully or not (Java `Future.isDone`).
    pub fn is_finished(&self) -> bool {
        self.0.is_finished()
    }
}

impl DownloadManager {
    pub fn new(
        download_cache: DownloadCache,
        downloader: Arc<dyn Downloader>,
        bzlmod_http_downloader: Arc<HttpDownloader>,
        event_handler: Arc<dyn ExtendedEventHandler>,
    ) -> Self {
        DownloadManager {
            download_cache,
            distdir: Vec::new(),
            rewriter: None,
            downloader,
            bzlmod_http_downloader,
            event_handler,
            disable_download: false,
            retries: 0,
            netrc_creds: None,
            credential_factory: Arc::new(|m| Arc::new(StaticCredentials::new(m.clone()))),
        }
    }

    pub fn set_download_cache(&mut self, download_cache: DownloadCache) {
        self.download_cache = download_cache;
    }

    pub fn set_distdir(&mut self, distdir: Vec<Path>) {
        self.distdir = distdir;
    }

    pub fn set_url_rewriter(&mut self, rewriter: Option<UrlRewriter>) {
        self.rewriter = rewriter;
    }

    pub fn set_disable_download(&mut self, disable_download: bool) {
        self.disable_download = disable_download;
    }

    pub fn set_retries(&mut self, retries: i32) {
        assert!(retries >= 0, "IllegalArgumentException: Invalid retries");
        self.retries = retries;
    }

    pub fn set_netrc_creds(&mut self, netrc_creds: Option<Arc<dyn Credentials>>) {
        self.netrc_creds = netrc_creds;
    }

    pub fn set_credential_factory(&mut self, credential_factory: CredentialFactory) {
        self.credential_factory = credential_factory;
    }

    /// Starts a download on a new thread (`startDownload`).
    ///
    /// PORT: Java submits to an executor and registers with a `Phaser` for cancellation; Blaze
    /// runs one thread per download and has no cancellation.
    pub fn start_download(
        self: &Arc<Self>,
        original_urls: Vec<JavaUri>,
        headers: Headers,
        auth_headers: HashMap<JavaUri, Headers>,
        checksum: Option<Checksum>,
        canonical_id: String,
        type_: Option<String>,
        output: Path,
        client_env: HashMap<String, String>,
        context: String,
        may_hardlink: bool,
    ) -> PendingDownload {
        let this = self.clone();
        PendingDownload(std::thread::spawn(move || {
            this.download(
                &original_urls,
                &headers,
                &auth_headers,
                checksum.as_ref(),
                &canonical_id,
                type_.as_deref(),
                &output,
                &client_env,
                &context,
                may_hardlink,
            )
        }))
    }

    // Java: com/google/devtools/build/lib/bazel/repository/downloader/DownloadManager.java:162 DownloadManager.finalizeDownload
    pub fn finalize_download(download: PendingDownload) -> Result<Path, IoException> {
        match download.0.join() {
            Ok(r) => r,
            Err(p) => std::panic::resume_unwind(p),
        }
    }

    /// Downloads a file to disk and returns its path (`downloadInExecutor`), consulting the
    /// repository cache and distdirs first when a checksum is given.
    ///
    /// `output` is the destination file if `type_` is absent, else the output directory.
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/DownloadManager.java:195 DownloadManager.downloadInExecutor
    pub fn download(
        &self,
        original_urls: &[JavaUri],
        headers: &Headers,
        auth_headers: &HashMap<JavaUri, Headers>,
        checksum: Option<&Checksum>,
        canonical_id: &str,
        type_: Option<&str>,
        output: &Path,
        client_env: &HashMap<String, String>,
        context: &str,
        may_hardlink: bool,
    ) -> Result<Path, IoException> {
        let mut rewritten_urls: Vec<JavaUri> = original_urls.to_vec();
        let mut rewritten_auth_headers = auth_headers.clone();
        if let Some(rewriter) = &self.rewriter {
            let mappings = rewriter.amend(original_urls);
            rewritten_urls = mappings.iter().map(|m| m.url.clone()).collect();
            rewritten_auth_headers =
                rewriter.update_auth_headers(&mappings, auth_headers, self.netrc_creds.as_deref());
        }
        // The "main" URL for this request, used for reporting, and the URL used to derive the
        // download's file name.
        let (main_url, file_name_url) = if rewritten_urls.is_empty() {
            let main = match type_ {
                Some(t) if !t.is_empty() => {
                    JavaUri::create(&format!("http://nonexistent.example.org/cacheprobe.{}", t))
                        .unwrap()
                }
                _ => JavaUri::create("http://nonexistent.example.org/cacheprobe").unwrap(),
            };
            let file_name = original_urls
                .first()
                .cloned()
                .unwrap_or_else(|| main.clone());
            (main, file_name)
        } else {
            (rewritten_urls[0].clone(), rewritten_urls[0].clone())
        };
        let destination = get_download_destination(&file_name_url, type_, output);
        let candidate_file_names = get_candidate_file_names(&main_url, &destination);
        let main_url_string = main_url.to_string();

        // Is set to true if the value should be cached by the checksum value provided
        let mut is_caching_by_provided_checksum = false;
        if let Some(checksum) = checksum {
            let cache_key = checksum.to_string();
            let cache_key_type = checksum.get_key_type();
            self.event_handler.post(CacheProgress::started(
                &main_url_string,
                format!("Checking in {} cache", cache_key_type),
            ));
            let current = download_cache::get_checksum(cache_key_type, &destination);
            self.event_handler
                .post(CacheProgress::finished(&main_url_string));
            if let Ok(current_checksum) = current
                && current_checksum == cache_key
            {
                // No need to download.
                return Ok(destination);
            }
            if self.download_cache.is_enabled() {
                is_caching_by_provided_checksum = true;
                if let Ok(Some(cached_destination)) = self.download_cache.get(
                    &cache_key,
                    &destination,
                    cache_key_type,
                    Some(canonical_id),
                    may_hardlink,
                ) {
                    // Cache hit!
                    self.event_handler.post(Arc::new(DownloadCacheHitEvent {
                        context: context.to_string(),
                        file_hash: cache_key.clone(),
                        uri: main_url.clone(),
                    }));
                    return Ok(cached_destination);
                }
            }
            if rewritten_urls.is_empty() {
                let mut message = "Cache miss and no url specified".to_string();
                if !original_urls.is_empty() {
                    message.push_str(" - ");
                    message.push_str(
                        &self
                            .get_rewriter_blocked_all_urls_message(original_urls)
                            .unwrap_or_else(|| "null".to_string()),
                    );
                }
                return Err(IoException::new(message));
            }
            for dir in &self.distdir {
                if !dir.exists() {
                    // This is not a warning; it is perfectly fine to have a common rc-file
                    // pointing to a volume that is sometimes, but not always mounted.
                    self.event_handler
                        .handle(&Event::info(format!("non-existent distdir {}", dir)));
                } else if !dir.is_directory() {
                    self.event_handler
                        .handle(&Event::warn(format!("distdir {} is not a directory", dir)));
                } else {
                    for name in &candidate_file_names {
                        let candidate = dir.get_relative_str(name);
                        self.event_handler.post(CacheProgress::started(
                            &main_url_string,
                            format!("Checking {} of {}", cache_key_type, candidate),
                        ));
                        // Not finding anything in a distdir is a normal case, so handle it
                        // absolutely quietly.
                        let matched = download_cache::get_checksum(cache_key_type, &candidate)
                            .is_ok_and(|c| c == cache_key);
                        self.event_handler
                            .post(CacheProgress::finished(&main_url_string));
                        if matched {
                            if is_caching_by_provided_checksum
                                && let Err(e) = self.download_cache.put(
                                    &cache_key,
                                    &candidate,
                                    cache_key_type,
                                    Some(canonical_id),
                                )
                            {
                                self.event_handler.handle(&Event::warn(format!(
                                    "Failed to copy {} to repository cache: {}",
                                    candidate,
                                    e.to_java_string()
                                )));
                            }
                            if let Some(parent) = destination.get_parent_directory() {
                                fs_utils::io(parent.create_directory_and_parents())?;
                            }
                            fs_utils::io(fs_utils::copy_file(&candidate, &destination))?;
                            return Ok(destination);
                        }
                    }
                }
            }
        }

        if self.disable_download {
            return Err(IoException::new(format!(
                "Failed to download {}: download is disabled.",
                context
            )));
        }
        if rewritten_urls.is_empty() && !original_urls.is_empty() {
            return Err(IoException::new(
                self.get_rewriter_blocked_all_urls_message(original_urls)
                    .unwrap_or_else(|| "null".to_string()),
            ));
        }

        let mut attempt = 0;
        loop {
            match self.downloader.download(
                &rewritten_urls,
                headers,
                (self.credential_factory)(&rewritten_auth_headers),
                checksum,
                canonical_id,
                &destination,
                self.event_handler.clone(),
                client_env,
                type_,
                context,
            ) {
                Ok(()) => break,
                Err(e) if e.kind.is_interrupted_io() => return Err(e),
                Err(e) => {
                    if !self.should_retry_download(&e, attempt) {
                        return Err(e);
                    }
                }
            }
            attempt += 1;
        }

        if is_caching_by_provided_checksum {
            let checksum = checksum.unwrap();
            self.download_cache.put(
                &checksum.to_string(),
                &destination,
                checksum.get_key_type(),
                Some(canonical_id),
            )?;
        } else if self.download_cache.is_enabled() {
            self.download_cache.put_computing_key(
                &destination,
                KeyType::SHA256,
                Some(canonical_id),
            )?;
        }
        Ok(destination)
    }

    // Java: com/google/devtools/build/lib/bazel/repository/downloader/DownloadManager.java:394 DownloadManager.shouldRetryDownload
    fn should_retry_download(&self, e: &IoException, attempt: i32) -> bool {
        if attempt >= self.retries {
            return false;
        }
        is_retryable_exception(e) || e.suppressed.iter().any(is_retryable_exception)
    }

    /// Downloads the contents of one URL into memory, for Bzlmod registry files. Ignores
    /// `--repository_disable_download`.
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/DownloadManager.java:435 DownloadManager.downloadAndReadOneUrlForBzlmod
    pub fn download_and_read_one_url_for_bzlmod(
        &self,
        original_url: &JavaUri,
        client_env: &HashMap<String, String>,
        checksum: Option<&Checksum>,
    ) -> Result<Vec<u8>, IoException> {
        if self.download_cache.is_enabled()
            && let Some(checksum) = checksum
        {
            let cache_key = checksum.to_string();
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
        let mut auth_headers: HashMap<JavaUri, Headers> = HashMap::new();
        let mut rewritten_urls = vec![original_url.clone()];
        if let Some(netrc) = &self.netrc_creds {
            // If the credentials extraction failed, we're letting bazel try without them.
            if let Ok(metadata) = netrc.get_request_metadata(original_url)
                && let Some((k, v)) = metadata.first()
            {
                let mut h = Headers::new();
                h.insert(k.clone(), vec![v[0].clone()]);
                auth_headers.insert(original_url.clone(), h);
            }
        }
        if let Some(rewriter) = &self.rewriter {
            let mappings = rewriter.amend(std::slice::from_ref(original_url));
            rewritten_urls = mappings.iter().map(|m| m.url.clone()).collect();
            auth_headers =
                rewriter.update_auth_headers(&mappings, &auth_headers, self.netrc_creds.as_deref());
        }
        if rewritten_urls.is_empty() {
            return Err(IoException::new(
                self.get_rewriter_blocked_all_urls_message(std::slice::from_ref(original_url))
                    .unwrap_or_else(|| "null".to_string()),
            ));
        }
        let mut attempt = 0;
        let content = loop {
            match self.bzlmod_http_downloader.download_and_read(
                &rewritten_urls,
                (self.credential_factory)(&auth_headers),
                checksum,
                self.event_handler.clone(),
                client_env,
            ) {
                Ok(c) => break c,
                Err(e) if e.kind.is_interrupted_io() => return Err(e),
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
                Some(c) => self.download_cache.put_bytes_with_key(
                    &c.to_string(),
                    &content,
                    c.get_key_type(),
                )?,
                None => self.download_cache.put_bytes(&content, KeyType::SHA256)?,
            }
        }
        Ok(content)
    }

    // Java: com/google/devtools/build/lib/bazel/repository/downloader/DownloadManager.java:521 DownloadManager.getRewriterBlockedAllUrlsMessage
    fn get_rewriter_blocked_all_urls_message(&self, original_urls: &[JavaUri]) -> Option<String> {
        let rewriter = self.rewriter.as_ref()?;
        let mut message = format!(
            "Configured URL rewriter blocked all URLs: {}",
            uri_list_to_string(original_urls)
        );
        if let Some(m) = rewriter.get_all_blocked_message() {
            message.push_str(" - ");
            message.push_str(m);
        }
        Some(message)
    }
}

// Java: com/google/devtools/build/lib/bazel/repository/downloader/DownloadManager.java:410 DownloadManager.isRetryableException
fn is_retryable_exception(e: &IoException) -> bool {
    e.kind == IoExceptionKind::ContentLengthMismatchException
        || e.kind.is_socket_exception()
        || e.kind == IoExceptionKind::UnknownHostException
}

/// The complement of a conservative range of characters valid for all reasonable file systems.
fn replace_fs_unsafe_chars(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || ".-_".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect()
}

// Java: com/google/devtools/build/lib/bazel/repository/downloader/DownloadManager.java:543 DownloadManager.getDownloadDestination
fn get_download_destination(url: &JavaUri, type_: Option<&str>, output: &Path) -> Path {
    let Some(type_) = type_ else {
        return output.clone();
    };
    let mut basename = get_url_base_name(url);
    if basename.is_empty() {
        basename = "temp".to_string();
    }
    if !type_.is_empty() {
        let suffix = format!(".{}", type_);
        if !basename.ends_with(&suffix) {
            basename.push_str(&suffix);
        }
    }
    // The basename may contain characters that aren't legal in a path with all file systems.
    output.get_relative_str(&replace_fs_unsafe_chars(&basename))
}

/// The file names to look for in the distdirs.
// Java: com/google/devtools/build/lib/bazel/repository/downloader/DownloadManager.java:566 DownloadManager.getCandidateFileNames
pub fn get_candidate_file_names(url: &JavaUri, destination: &Path) -> IndexSet<String> {
    let url_base_name = get_url_base_name(url);
    let mut s = IndexSet::new();
    if !url_base_name.is_empty() && url_base_name != destination.get_base_name() {
        s.insert(url_base_name);
    }
    s.insert(destination.get_base_name().to_string());
    s
}

// Java: com/google/devtools/build/lib/bazel/repository/downloader/DownloadManager.java:575 DownloadManager.getUrlBaseName
fn get_url_base_name(url: &JavaUri) -> String {
    let mut path = url.get_path();
    if path.is_none() && url.is_opaque() {
        // Match URL#getPath() behavior for opaque file URIs such as file:../archive.tgz.
        let mut raw_path = url.get_raw_scheme_specific_part().to_string();
        if let Some(q) = raw_path.find('?') {
            raw_path.truncate(q);
        }
        path = Some(if raw_path.is_empty() {
            String::new()
        } else {
            JavaUri::create(&format!("{}:{}", url.get_scheme().unwrap_or(""), raw_path))
                .map(|u| u.get_scheme_specific_part())
                .unwrap_or(raw_path)
        });
    }
    match path {
        None => String::new(),
        Some(p) => PathFragment::create(&p).get_base_name().to_string(),
    }
}
