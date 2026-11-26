//! Port of `lib/remote/RepositoryRemoteHelpersFactoryImpl.java`.

use std::sync::Arc;

use blaze_repo_rules::repository::remote_helpers::{
    RemoteRepoContentsCache, RepositoryRemoteHelpersFactory,
};
use blaze_sky_loading::directories::LoadingDirectories;

use crate::combined_cache::CombinedCache;
use crate::remote_repo_contents_cache_impl::RemoteRepoContentsCacheImpl;

/// Factory for `RemoteRepositoryRemoteExecutor` and [`RemoteRepoContentsCacheImpl`].
///
/// PORT: `createExecutor` (remote execution of repository rules) is not part of the factory yet
/// (see `blaze_repo_rules::repository::remote_helpers::RepositoryRemoteHelpersFactory`).
// Java: com/google/devtools/build/lib/remote/RepositoryRemoteHelpersFactoryImpl.java:26 RepositoryRemoteHelpersFactoryImpl
pub struct RepositoryRemoteHelpersFactoryImpl {
    directories: LoadingDirectories,
    cache: Arc<CombinedCache>,
    build_request_id: String,
    command_id: String,
    accept_cached: bool,
    upload_local_results: bool,
    verbose_failures: bool,
}

impl RepositoryRemoteHelpersFactoryImpl {
    // Java: com/google/devtools/build/lib/remote/RepositoryRemoteHelpersFactoryImpl.java:39 RepositoryRemoteHelpersFactoryImpl.<init>
    pub fn new(
        directories: LoadingDirectories,
        cache: Arc<CombinedCache>,
        build_request_id: String,
        command_id: String,
        accept_cached: bool,
        upload_local_results: bool,
        verbose_failures: bool,
    ) -> RepositoryRemoteHelpersFactoryImpl {
        RepositoryRemoteHelpersFactoryImpl {
            directories,
            cache,
            build_request_id,
            command_id,
            accept_cached,
            upload_local_results,
            verbose_failures,
        }
    }
}

impl RepositoryRemoteHelpersFactory for RepositoryRemoteHelpersFactoryImpl {
    // Java: com/google/devtools/build/lib/remote/RepositoryRemoteHelpersFactoryImpl.java:78 RepositoryRemoteHelpersFactoryImpl.createRepoContentsCache
    fn create_repo_contents_cache(&self) -> Option<Arc<dyn RemoteRepoContentsCache>> {
        Some(Arc::new(RemoteRepoContentsCacheImpl::new(
            self.directories.clone(),
            self.cache.clone(),
            self.build_request_id.clone(),
            self.command_id.clone(),
            self.accept_cached,
            self.upload_local_results,
            self.verbose_failures,
        )))
    }
}
