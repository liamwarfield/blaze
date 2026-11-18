//! Integration seams: what `lib/remote` reads from classes that live in crates this one cannot
//! change or depend on. Each field is `None` until the integrator (the execution phase's setup,
//! or a test harness) provides it; a `None` hook that is needed fails loudly with `unported!`.
//!
//! These are the additive changes requested from the owners of `blaze_exec` /
//! `blaze_sky_exec` (docs/remote.md, "Hooks"); once they exist the hook becomes a direct call.

use std::io;
use std::sync::Arc;
use std::time::SystemTime;

use blaze_exec::actions::ActionExecutionMetadata;
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

/// `ActionExecutionMetadata.mayModifySpawnOutputsAfterExecution()` (true for
/// `TestRunnerAction`, `JavaCompileAction`, `JavaHeaderCompileAction`).
// Java: com/google/devtools/build/lib/actions/ActionExecutionMetadata.java:125 ActionExecutionMetadata.mayModifySpawnOutputsAfterExecution
pub type MayModifySpawnOutputsAfterExecution =
    Arc<dyn Fn(&dyn ActionExecutionMetadata) -> bool + Send + Sync>;

/// The hooks of one build.
#[derive(Clone, Default)]
pub struct RemoteHooks {
    /// Missing hook: `SpawnExecutionContext.getActionFileSystem()` as a
    /// `RemoteActionFileSystem`.
    pub remote_output_injector: Option<Arc<dyn RemoteOutputInjector>>,
    /// Missing hook: `blaze_exec::actions::ActionExecutionMetadata::may_modify_spawn_outputs_after_execution`.
    pub may_modify_spawn_outputs_after_execution: Option<MayModifySpawnOutputsAfterExecution>,
}

impl RemoteHooks {
    /// No hook provided.
    pub fn new() -> RemoteHooks {
        RemoteHooks::default()
    }

    /// `spawnOwner.mayModifySpawnOutputsAfterExecution()`.
    pub fn may_modify_spawn_outputs_after_execution(
        &self,
        spawn_owner: &dyn ActionExecutionMetadata,
    ) -> bool {
        match &self.may_modify_spawn_outputs_after_execution {
            Some(f) => f(spawn_owner),
            None => blaze_util::unported!(
                "ActionExecutionMetadata.mayModifySpawnOutputsAfterExecution (missing hook: blaze_exec::actions::ActionExecutionMetadata::may_modify_spawn_outputs_after_execution; RemoteHooks.may_modify_spawn_outputs_after_execution is not set)"
            ),
        }
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
