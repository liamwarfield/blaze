//! Port of `com.google.devtools.build.lib.bazel.repository.cache`.

pub mod download_cache;
pub mod local_repo_contents_cache;
pub mod repository_cache;

pub use download_cache::{DownloadCache, KeyType};
pub use local_repo_contents_cache::{CandidateRepo, LocalRepoContentsCache};
pub use repository_cache::RepositoryCache;

use std::sync::Arc;

/// Event reporting about cache hits for download requests.
// Java: com/google/devtools/build/lib/bazel/repository/cache/DownloadCacheHitEvent.java:21 DownloadCacheHitEvent
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadCacheHitEvent {
    pub context: String,
    pub file_hash: String,
    pub uri: crate::uri::JavaUri,
}

impl blaze_events::handler::Postable for DownloadCacheHitEvent {}

impl DownloadCacheHitEvent {
    pub fn into_postable(self) -> Arc<dyn blaze_events::handler::Postable> {
        Arc::new(self)
    }
}
