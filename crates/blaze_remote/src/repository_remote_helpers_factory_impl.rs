//! Port of `lib/remote/RepositoryRemoteHelpersFactoryImpl.java`.

use std::sync::Arc;

use blaze_repo_rules::repository::remote_helpers::{
    RemoteRepoContentsCache, RepositoryRemoteExecutor, RepositoryRemoteHelpersFactory,
};
use blaze_sky_loading::directories::LoadingDirectories;
use blaze_vfs::Path;

use crate::combined_cache::CombinedCache;
use crate::common::RemoteExecutionClient;
use crate::remote_execution_cache::RemoteExecutionCache;
use crate::remote_repo_contents_cache_impl::RemoteRepoContentsCacheImpl;
use crate::remote_repository_remote_executor::RemoteRepositoryRemoteExecutor;

/// What `createExecutor` needs when remote execution is configured: Java's
/// `(RemoteExecutionCache) cache` and `remoteExecutor`, and the exec root that the port's
/// `MerkleTreeComputer` takes (see [`RemoteRepositoryRemoteExecutor`]).
pub struct RemoteExecutionHelpers {
    pub cache: Arc<RemoteExecutionCache>,
    pub remote_executor: Arc<dyn RemoteExecutionClient>,
    pub exec_root: Path,
}

/// Factory for [`RemoteRepositoryRemoteExecutor`] and [`RemoteRepoContentsCacheImpl`].
// Java: com/google/devtools/build/lib/remote/RepositoryRemoteHelpersFactoryImpl.java:26 RepositoryRemoteHelpersFactoryImpl
pub struct RepositoryRemoteHelpersFactoryImpl {
    directories: LoadingDirectories,
    cache: Arc<CombinedCache>,
    /// Java `remoteExecutor` (null without remote execution), with the cache it needs.
    remote_execution: Option<RemoteExecutionHelpers>,
    build_request_id: String,
    command_id: String,
    workspace_name: String,
    remote_instance_name: String,
    accept_cached: bool,
    upload_local_results: bool,
    verbose_failures: bool,
}

impl RepositoryRemoteHelpersFactoryImpl {
    // Java: com/google/devtools/build/lib/remote/RepositoryRemoteHelpersFactoryImpl.java:39 RepositoryRemoteHelpersFactoryImpl.<init>
    pub fn new(
        directories: LoadingDirectories,
        cache: Arc<CombinedCache>,
        remote_execution: Option<RemoteExecutionHelpers>,
        build_request_id: String,
        command_id: String,
        workspace_name: String,
        remote_instance_name: String,
        accept_cached: bool,
        upload_local_results: bool,
        verbose_failures: bool,
    ) -> RepositoryRemoteHelpersFactoryImpl {
        RepositoryRemoteHelpersFactoryImpl {
            directories,
            cache,
            remote_execution,
            build_request_id,
            command_id,
            workspace_name,
            remote_instance_name,
            accept_cached,
            upload_local_results,
            verbose_failures,
        }
    }
}

impl RepositoryRemoteHelpersFactory for RepositoryRemoteHelpersFactoryImpl {
    // Java: com/google/devtools/build/lib/remote/RepositoryRemoteHelpersFactoryImpl.java:62 RepositoryRemoteHelpersFactoryImpl.createExecutor
    fn create_executor(&self) -> Option<Arc<dyn RepositoryRemoteExecutor>> {
        let remote_execution = self.remote_execution.as_ref()?;
        Some(Arc::new(RemoteRepositoryRemoteExecutor::new(
            remote_execution.cache.clone(),
            remote_execution.remote_executor.clone(),
            remote_execution.cache.digest_util.clone(),
            &self.build_request_id,
            &self.command_id,
            &self.workspace_name,
            &self.remote_instance_name,
            self.accept_cached,
            remote_execution.exec_root.clone(),
        )))
    }

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
