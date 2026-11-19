//! Integration seams: what `lib/remote` reads from classes that live in crates above this one.
//! Each field is `None` until the integrator (`blaze_buildtool`'s `ExecutionTool`, or a test
//! harness) provides it; a `None` hook that is needed fails loudly with `unported!`.

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

/// The hooks of one build.
#[derive(Clone, Default)]
pub struct RemoteHooks {
    /// `SpawnExecutionContext.getActionFileSystem()` as a `RemoteActionFileSystem`: the
    /// executor's remote output tree (`blaze_sky_exec::remote_output_tree`), which
    /// `blaze_sky_exec` provides above this crate.
    pub remote_output_injector: Option<Arc<dyn RemoteOutputInjector>>,
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
