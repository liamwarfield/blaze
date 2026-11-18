//! Port of `RepositoryCache`: the `--repository_cache` directory, holding the
//! [`DownloadCache`] (`content_addressable/`) and the [`LocalRepoContentsCache`] (`contents/`).

use blaze_vfs::Path;

use super::download_cache::DownloadCache;
use super::local_repo_contents_cache::LocalRepoContentsCache;

// Repository cache subdirectories
pub const CAS_DIR: &str = "content_addressable";
pub const CONTENTS_DIR: &str = "contents";

/// A cache directory related to repositories, containing both the [`DownloadCache`] and the
/// [`LocalRepoContentsCache`].
// Java: com/google/devtools/build/lib/bazel/repository/cache/RepositoryCache.java:26 RepositoryCache
#[derive(Debug, Default)]
pub struct RepositoryCache {
    download_cache: DownloadCache,
    repo_contents_cache: LocalRepoContentsCache,
    path: Option<Path>,
}

impl RepositoryCache {
    pub fn new() -> Self {
        Self::default()
    }

    // Java: com/google/devtools/build/lib/bazel/repository/cache/RepositoryCache.java:40 RepositoryCache.setPath
    pub fn set_path(&mut self, path: Option<Path>) {
        match &path {
            Some(p) => {
                self.download_cache
                    .set_path(Some(p.get_relative_str(CAS_DIR)));
                self.repo_contents_cache
                    .set_path(Some(p.get_relative_str(CONTENTS_DIR)));
            }
            None => {
                self.download_cache.set_path(None);
                self.repo_contents_cache.set_path(None);
            }
        }
        self.path = path;
    }

    pub fn get_download_cache(&self) -> &DownloadCache {
        &self.download_cache
    }

    pub fn get_download_cache_mut(&mut self) -> &mut DownloadCache {
        &mut self.download_cache
    }

    pub fn get_repo_contents_cache(&self) -> &LocalRepoContentsCache {
        &self.repo_contents_cache
    }

    pub fn get_repo_contents_cache_mut(&mut self) -> &mut LocalRepoContentsCache {
        &mut self.repo_contents_cache
    }

    pub fn get_path(&self) -> Option<&Path> {
        self.path.as_ref()
    }
}
