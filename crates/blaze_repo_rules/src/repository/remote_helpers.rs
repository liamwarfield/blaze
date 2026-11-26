//! Ports of `lib/runtime/RemoteRepoContentsCache.java` and
//! `lib/runtime/RepositoryRemoteHelpersFactory.java`, and of the runtime-level
//! `RepositoryRemoteHelpersFactory` delegate (`RemoteModule.RepositoryRemoteHelpersFactoryDelegate`
//! as `BlazeRuntime.getRepositoryHelpersFactory()` returns it).
//!
//! PORT: the interfaces live in `lib/runtime` in Java; here they live with their caller (the
//! repository fetch function), and `blaze_remote` implements them. Java's runtime holds one
//! delegate that `RemoteModule.beforeCommand` initializes and `afterCommand` resets; this module
//! holds it ([`init_repository_remote_helpers_factory`], [`reset_repository_remote_helpers_factory`],
//! [`create_remote_repo_contents_cache`]).

use std::sync::Arc;

use blaze_cmdline::RepositoryName;
use blaze_events::ExtendedEventHandler;
use blaze_sky_loading::io::IOException;
use blaze_skyframe::{Environment, Interrupted};
use blaze_vfs::Path;
use parking_lot::RwLock;

/// A remote cache for the contents of external repositories.
// Java: com/google/devtools/build/lib/runtime/RemoteRepoContentsCache.java:25 RemoteRepoContentsCache
pub trait RemoteRepoContentsCache: Send + Sync {
    /// Adds a repository that has been fetched locally to the remote cache.
    // Java: com/google/devtools/build/lib/runtime/RemoteRepoContentsCache.java:27 RemoteRepoContentsCache.addToCache
    fn add_to_cache(
        &self,
        repo_name: &RepositoryName,
        fetched_repo_dir: &Path,
        fetched_repo_marker_file: &Path,
        predeclared_input_hash: &str,
        reporter: &dyn ExtendedEventHandler,
    ) -> Result<(), Interrupted>;

    /// Retrieves a repository from the remote cache if possible.
    ///
    /// Callers have to check `env.values_missing()` after this method returns.
    ///
    /// Returns true if there was a cache hit and the repository has been fetched into the given
    /// directory. The inner `Err` is Java's `IOException`.
    // Java: com/google/devtools/build/lib/runtime/RemoteRepoContentsCache.java:44 RemoteRepoContentsCache.lookupCache
    fn lookup_cache(
        &self,
        repo_name: &RepositoryName,
        repo_dir: &Path,
        predeclared_input_hash: &str,
        env: &mut Environment<'_>,
    ) -> Result<Result<bool, IOException>, Interrupted>;
}

/// Factory for `RepositoryRemoteExecutor` and [`RemoteRepoContentsCache`].
///
/// PORT: `createExecutor` (remote execution of repository rules) is not part of this trait yet.
// Java: com/google/devtools/build/lib/runtime/RepositoryRemoteHelpersFactory.java:21 RepositoryRemoteHelpersFactory
pub trait RepositoryRemoteHelpersFactory: Send + Sync {
    /// Returns a new [`RemoteRepoContentsCache`] or `None`.
    // Java: com/google/devtools/build/lib/runtime/RepositoryRemoteHelpersFactory.java:29 RepositoryRemoteHelpersFactory.createRepoContentsCache
    fn create_repo_contents_cache(&self) -> Option<Arc<dyn RemoteRepoContentsCache>>;
}

/// `RepositoryRemoteHelpersFactoryDelegate.delegate`.
static DELEGATE: RwLock<Option<Arc<dyn RepositoryRemoteHelpersFactory>>> = RwLock::new(None);

/// `repositoryRemoteHelpersFactoryDelegate.init(delegate)`.
// Java: com/google/devtools/build/lib/remote/RemoteModule.java:1397 RepositoryRemoteHelpersFactoryDelegate.init
pub fn init_repository_remote_helpers_factory(delegate: Arc<dyn RepositoryRemoteHelpersFactory>) {
    let mut current = DELEGATE.write();
    blaze_util::check_state!(current.is_none());
    *current = Some(delegate);
}

/// `repositoryRemoteHelpersFactoryDelegate.reset()`.
// Java: com/google/devtools/build/lib/remote/RemoteModule.java:1402 RepositoryRemoteHelpersFactoryDelegate.reset
pub fn reset_repository_remote_helpers_factory() {
    *DELEGATE.write() = None;
}

/// `env.getRuntime().getRepositoryHelpersFactory().createRepoContentsCache()`: `None` when no
/// delegate is set.
// Java: com/google/devtools/build/lib/remote/RemoteModule.java:1419 RepositoryRemoteHelpersFactoryDelegate.createRepoContentsCache
pub fn create_remote_repo_contents_cache() -> Option<Arc<dyn RemoteRepoContentsCache>> {
    let delegate = DELEGATE.read().clone()?;
    delegate.create_repo_contents_cache()
}

/// `RemoteExternalOverlayFileSystem.ensureMaterialized`, as `StarlarkBaseExternalContext` calls
/// it when the output base is on that file system.
///
/// PORT: the overlay is `blaze_remote`'s, which this crate cannot name; the runtime, which
/// installs it as the file system of the output base at startup, registers it here
/// ([`set_remote_repo_materializer`]), so "a materializer is registered" is Java's
/// `directories.getOutputBase().getFileSystem() instanceof RemoteExternalOverlayFileSystem`.
pub trait RemoteRepoMaterializer: Send + Sync {
    // Java: com/google/devtools/build/lib/remote/RemoteExternalOverlayFileSystem.java:395 RemoteExternalOverlayFileSystem.ensureMaterialized
    fn ensure_materialized(
        &self,
        repo: &RepositoryName,
        reporter: &dyn ExtendedEventHandler,
    ) -> std::io::Result<()>;
}

static MATERIALIZER: RwLock<Option<Arc<dyn RemoteRepoMaterializer>>> = RwLock::new(None);

/// Registers the overlay file system of the output base (see [`RemoteRepoMaterializer`]).
pub fn set_remote_repo_materializer(materializer: Option<Arc<dyn RemoteRepoMaterializer>>) {
    *MATERIALIZER.write() = materializer;
}

/// The registered overlay file system, if the output base is on one.
pub fn remote_repo_materializer() -> Option<Arc<dyn RemoteRepoMaterializer>> {
    MATERIALIZER.read().clone()
}
