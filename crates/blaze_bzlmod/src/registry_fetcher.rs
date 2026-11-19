//! A [`RegistryFetcher`](blaze_bzlmod_model::RegistryFetcher) over the downloader and the
//! repository cache (`DownloadManager.downloadAndReadOneUrlForBzlmod`).
//!
//! [`DownloadManagerRegistryFetcher`] is the seam over `blaze_repo_fetch`'s `DownloadManager`:
//! the one download manager `BazelRepositoryModule.beforeCommand` builds and hands to
//! `ModuleFileFunction`, `RepoSpecFunction`, `YankedVersionsFunction` and the repository
//! functions alike. Registry files therefore see the same `--downloader_config` rewriter,
//! `.netrc` credentials, `--http_*` options and proxy handling as repository rule downloads.

use std::sync::Arc;

use blaze_bzlmod_model::registry::{RegistryException, RegistryFetcher};
use blaze_bzlmod_model::{Checksum, JavaUri, KeyType};
use blaze_events::ExtendedEventHandler;
use blaze_repo_fetch::downloader::download_manager::DownloadManager;
use indexmap::IndexMap;

/// The repository cache's download (content-addressable) directory (`RepositoryCache.CAS_DIR`).
// Java: com/google/devtools/build/lib/bazel/repository/cache/RepositoryCache.java:26 RepositoryCache.CAS_DIR
pub const CAS_DIR: &str = "content_addressable";

/// The registry seam over `blaze_repo_fetch`'s `DownloadManager`. Converts this crate's
/// `JavaUri` / `Checksum` to `blaze_repo_fetch`'s by their string forms.
// Java: com/google/devtools/build/lib/bazel/repository/downloader/DownloadManager.java:435 DownloadManager.downloadAndReadOneUrlForBzlmod
pub struct DownloadManagerRegistryFetcher {
    download_manager: Arc<DownloadManager>,
}

impl DownloadManagerRegistryFetcher {
    /// The fetcher over the command's download manager (Java `setDownloadManager(downloadManager)`
    /// on `ModuleFileFunction`, `RepoSpecFunction` and `YankedVersionsFunction`).
    // Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:284 BazelRepositoryModule.beforeCommand
    pub fn new(download_manager: Arc<DownloadManager>) -> Self {
        DownloadManagerRegistryFetcher { download_manager }
    }

    /// A download manager with only what the dev drivers and tests configure: the repository
    /// cache at `repository_cache` (`None` disables it), the given HTTP downloader for both the
    /// delegating downloader and the bzlmod downloader, and `retries`
    /// (`--experimental_repository_downloader_retries`; applied only when non-negative, as
    /// `BazelRepositoryModule.beforeCommand` does). The URL rewriter, `.netrc` credentials and
    /// distdirs are the download manager's defaults (none).
    pub fn create(
        repository_cache: Option<&std::path::Path>,
        http_downloader: blaze_repo_fetch::downloader::HttpDownloader,
        event_handler: Arc<dyn ExtendedEventHandler>,
        retries: i32,
    ) -> Self {
        use blaze_repo_fetch::cache::RepositoryCache;
        use blaze_repo_fetch::downloader::{DelegatingDownloader, Downloader};
        use blaze_vfs::unix_file_system::UnixFileSystem;
        use blaze_vfs::{DigestHashFunction, Path, PathFragment};

        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        let mut repository_cache_value = RepositoryCache::new();
        repository_cache_value
            .set_path(repository_cache.map(|p| {
                Path::create(PathFragment::create(&crate::skyframe::path_string(p)), &fs)
            }));
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
        if retries >= 0 {
            download_manager.set_retries(retries);
        }
        DownloadManagerRegistryFetcher {
            download_manager: Arc::new(download_manager),
        }
    }

    pub fn get_download_manager(&self) -> &Arc<DownloadManager> {
        &self.download_manager
    }
}

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

/// Maps the download path's exception to the registry seam's error: `FileNotFoundException` is
/// [`RegistryException::NotFound`], an interrupt is [`RegistryException::Interrupted`], and any
/// other `IOException` is [`RegistryException::Io`] with its message.
fn from_repo_fetch_exception(e: blaze_repo_fetch::errors::IoException) -> RegistryException {
    use blaze_repo_fetch::errors::IoExceptionKind as RfKind;
    match e.kind {
        RfKind::FileNotFoundException => RegistryException::NotFound(e.get_message()),
        // DownloadManager turns an InterruptedIOException into an InterruptedException.
        RfKind::InterruptedIOException => RegistryException::Interrupted,
        _ => RegistryException::Io(e.get_message()),
    }
}

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
