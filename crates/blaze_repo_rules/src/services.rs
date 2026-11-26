//! What the repository SkyFunctions are constructed with.
//!
//! Java: the constructor arguments and setters of `RepositoryFetchFunction` and
//! `SingleExtensionEvalFunction` (`BlazeDirectories`, the repo env suppliers, `DownloadManager`,
//! `--experimental_scale_timeouts`, `ProcessWrapper`, the `LocalRepoContentsCache`), set up by
//! `BazelRepositoryModule.beforeCommand`.

use std::sync::Arc;
use std::time::Duration;

use blaze_repo_fetch::cache::local_repo_contents_cache::LocalRepoContentsCache;
use blaze_repo_fetch::downloader::download_manager::DownloadManager;
use blaze_sky_loading::directories::LoadingDirectories;
use blaze_vfs::Path;
use indexmap::IndexMap;

/// Shared configuration of the repository and module extension SkyFunctions.
pub struct RepoServices {
    /// Java `BlazeDirectories` (workspace, output base, install base).
    pub directories: LoadingDirectories,
    /// Java `CommandEnvironment.getRepoEnv()` (`repoEnvSupplier`): the client environment
    /// (filtered by `--incompatible_strict_repo_env`) with `--repo_env` / `--action_env` applied.
    pub repo_env: Arc<IndexMap<String, String>>,
    /// Java `CommandEnvironment.getNonstrictRepoEnv()` (`nonstrictRepoEnvSupplier`).
    pub nonstrict_repo_env: Arc<IndexMap<String, String>>,
    /// Java `downloadManager`.
    pub download_manager: Option<Arc<DownloadManager>>,
    /// Java `timeoutScaling` (`--experimental_scale_timeouts`).
    pub timeout_scaling: f64,
    /// Java `ProcessWrapper`: `<install_base>/process-wrapper` when it exists.
    pub process_wrapper: Option<Path>,
    /// Java `ProcessWrapper.killDelay`: `LocalExecutionOptions.getLocalSigkillGraceSeconds()`
    /// (`--local_termination_grace_seconds`), or `None` (Java null) when the command has no
    /// `LocalExecutionOptions`; then no `--kill_delay` argument is passed to the process-wrapper.
    // Java: com/google/devtools/build/lib/runtime/ProcessWrapper.java:72 ProcessWrapper.fromCommandEnvironment
    pub process_wrapper_kill_delay: Option<Duration>,
    /// Java `ProcessWrapper.gracefulSigterm`: `LocalExecutionOptions.processWrapperGracefulSigterm`
    /// (`--experimental_process_wrapper_graceful_sigterm`), false when the command has no
    /// `LocalExecutionOptions`.
    // Java: com/google/devtools/build/lib/runtime/ProcessWrapper.java:72 ProcessWrapper.fromCommandEnvironment
    pub process_wrapper_graceful_sigterm: bool,
    /// Java `RepositoryFetchFunction.repoContentsCache`.
    pub repo_contents_cache: Arc<LocalRepoContentsCache>,
    /// Java `RepositoryFetchFunction.remoteRepoContentsCache` (`setRemoteRepoContentsCache`).
    pub remote_repo_contents_cache:
        Option<Arc<dyn crate::repository::remote_helpers::RemoteRepoContentsCache>>,
    /// Java `RepositoryFetchFunction.repositoryRemoteExecutor` (`setRepositoryRemoteExecutor`).
    pub remote_executor:
        Option<Arc<dyn crate::repository::remote_helpers::RepositoryRemoteExecutor>>,
}

impl RepoServices {
    /// `<output_base>/external`.
    // Java: com/google/devtools/build/lib/bazel/repository/RepositoryUtils.java:74 RepositoryUtils.getExternalRepositoryDirectory
    pub fn external_directory(&self) -> Path {
        self.directories.get_external_repository_directory()
    }

    /// Java `BlazeDirectories.getEmbeddedBinariesRoot()`: the install base.
    // Java: com/google/devtools/build/lib/analysis/BlazeDirectories.java:202 BlazeDirectories.getEmbeddedBinariesRoot
    pub fn embedded_binaries_root(&self) -> &Path {
        &self.directories.install_base
    }
}
