//! Integration seams: what `lib/remote` reads from classes that live in crates this one cannot
//! change or depend on. Each field is `None` until the integrator (the execution phase's setup,
//! or a test harness) provides it; a `None` hook that is needed fails loudly with `unported!`.
//!
//! These are the additive changes requested from the owners of `blaze_exec` /
//! `blaze_sky_exec` (docs/remote.md, "Hooks"); once they exist the hook becomes a direct call.

use std::io;
use std::sync::Arc;
use std::time::SystemTime;

use blaze_vfs::PathFragment;

/// The part of `RemoteActionFileSystem` that `RemoteExecutionService.downloadOutputs` writes
/// to: the metadata of an output that is not downloaded (or is only kept in memory) is injected
/// into the action file system, so that the output metadata store reports a remote file.
// Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:308 RemoteActionFileSystem.injectRemoteFile
pub trait RemoteOutputInjector: Send + Sync {
    /// `injectRemoteFile(path, digest, size, expirationTime, inMemoryOutput)`; `path` is the
    /// absolute path of the output under the exec root.
    fn inject_remote_file(
        &self,
        path: &PathFragment,
        digest: Vec<u8>,
        size: i64,
        expiration_time: SystemTime,
        in_memory_output: bool,
    ) -> io::Result<()>;
}

/// The build's remote output tree receives the injected metadata
/// (`RemoteActionFileSystem.injectRemoteFile`).
// Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:308 RemoteActionFileSystem.injectRemoteFile
impl RemoteOutputInjector for blaze_exec::exec::action_execution_context::RemoteOutputTree {
    fn inject_remote_file(
        &self,
        path: &PathFragment,
        digest: Vec<u8>,
        size: i64,
        expiration_time: SystemTime,
        in_memory_output: bool,
    ) -> io::Result<()> {
        if !self.is_attached() {
            // No output metadata store consults the tree (`blaze_sky_exec`'s
            // `ActionOutputMetadataStore` stands for Java's stat through the
            // RemoteActionFileSystem), so the output would be reported as not created. This is
            // an IOException of the download: a cache hit becomes a miss with the warning
            // "Remote Cache: ...", a remote execution fails.
            return Err(io::Error::other(
                "not ported yet: RemoteActionFileSystem.injectRemoteFile (missing hook: ActionOutputMetadataStore does not consult the RemoteOutputTree of the build)",
            ));
        }
        // `Instant` as `RemoteFileArtifactValue` keeps it: epoch milliseconds.
        let expiration_time_millis = match expiration_time.duration_since(SystemTime::UNIX_EPOCH) {
            Ok(d) => i64::try_from(d.as_millis()).unwrap_or(i64::MAX),
            Err(e) => -i64::try_from(e.duration().as_millis()).unwrap_or(i64::MAX),
        };
        let metadata =
            blaze_exec::actions::FileArtifactValue::create_for_remote_file_with_materialization_data(
                digest,
                size,
                /* location_index= */ 1,
                Some(expiration_time_millis),
                in_memory_output,
            );
        self.inject_file(path.clone(), metadata);
        Ok(())
    }
}

/// The hooks of one build.
#[derive(Clone, Default)]
pub struct RemoteHooks {
    /// Missing hook: `SpawnExecutionContext.getActionFileSystem()` as a
    /// `RemoteActionFileSystem`.
    pub remote_output_injector: Option<Arc<dyn RemoteOutputInjector>>,
    /// `env.getSkyframeExecutor().getEvaluator()` as `RemoteOutputChecker
    /// .maybeInvalidateSkyframeValues` uses it: `memoizingEvaluator.delete(predicate)` over the
    /// server's graph, the predicate taking the key's function name. `None`: no graph is kept
    /// (nothing to delete).
    #[allow(clippy::type_complexity)]
    pub skyframe_delete_by_function_name:
        Option<Arc<dyn Fn(Arc<dyn Fn(&str) -> bool + Send + Sync>) + Send + Sync>>,
}

impl RemoteHooks {
    /// No hook provided.
    pub fn new() -> RemoteHooks {
        RemoteHooks::default()
    }

    /// `remoteActionFileSystem.injectRemoteFile(...)`.
    pub fn inject_remote_file(
        &self,
        path: &PathFragment,
        digest: Vec<u8>,
        size: i64,
        expiration_time: SystemTime,
        in_memory_output: bool,
    ) -> io::Result<()> {
        match &self.remote_output_injector {
            Some(injector) => {
                injector.inject_remote_file(path, digest, size, expiration_time, in_memory_output)
            }
            None => blaze_util::unported!(
                "RemoteExecutionService.downloadOutputs (missing hook: action.getSpawnExecutionContext().getActionFileSystem() as RemoteActionFileSystem, then remoteActionFileSystem.injectRemoteFile(path, digest, size, expirationTime, inMemoryOutput); RemoteHooks.remote_output_injector is not set)"
            ),
        }
    }
}
