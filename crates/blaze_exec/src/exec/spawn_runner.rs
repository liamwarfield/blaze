//! Port of `lib/exec/SpawnRunner.java` and the progress events it reports
//! (`SpawnSchedulingEvent`, `SpawnExecutingEvent`, `SpawnCheckingCacheEvent`).

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use blaze_protos::tools::protos::Digest;
use blaze_vfs::{Path, PathFragment};
use indexmap::IndexMap;

use crate::actions::{
    ExecException, InputMapping, InputMetadataProvider, Spawn, SpawnResult, SpawnResultBuilder,
};
use crate::exec::tree_deleter::TreeDeleter;
use crate::util::FileOutErr;

/// `SpawnRunner.ProgressStatus` events.
// Java: com/google/devtools/build/lib/exec/SpawnRunner.java:115 SpawnRunner.ProgressStatus
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProgressStatus {
    /// `SpawnSchedulingEvent.create(runnerName)`.
    Scheduling(String),
    /// `SpawnExecutingEvent.create(runnerName)`.
    Executing(String),
    /// `SpawnCheckingCacheEvent.create(runnerName)`.
    CheckingCache(String),
}

/// `SpawnRunner.SpawnExecutionContext`: what a runner may ask of the strategy that called it.
///
/// Seam: implemented by `AbstractSpawnStrategy` (here) on top of an action execution context
/// supplied by `blaze_sky_exec`; the replay harness has its own implementation.
// Java: com/google/devtools/build/lib/exec/SpawnRunner.java:135 SpawnRunner.SpawnExecutionContext
pub trait SpawnExecutionContext: Send + Sync {
    /// `getId()`: unique per spawn for one runner.
    fn get_id(&self) -> i32;
    /// `setDigest(Digest)`.
    fn set_digest(&self, digest: Digest);
    /// `getDigest()`.
    fn get_digest(&self) -> Option<Digest>;
    /// `prefetchInputsAndWait()`.
    fn prefetch_inputs_and_wait(&self) -> Result<(), ExecException>;
    /// `getInputMetadataProvider()`.
    fn get_input_metadata_provider(&self) -> &dyn InputMetadataProvider;
    /// `lockOutputFiles(exitCode, errorMessage, outErr)`.
    fn lock_output_files(
        &self,
        exit_code: i32,
        error_message: &str,
        out_err: &FileOutErr,
    ) -> Result<(), ExecException>;
    /// `speculating()`.
    fn speculating(&self) -> bool;
    /// `getTimeout()`.
    fn get_timeout(&self) -> Duration;
    /// `getFileOutErr()`.
    fn get_file_out_err(&self) -> &FileOutErr;
    /// `getInputMapping(baseDirectory, willAccessRepeatedly)`.
    fn get_input_mapping(
        &self,
        base_directory: &PathFragment,
        will_access_repeatedly: bool,
    ) -> Arc<InputMapping>;
    /// `report(ProgressStatus)`.
    fn report(&self, progress: ProgressStatus);
    /// `isRewindingEnabled()`.
    fn is_rewinding_enabled(&self) -> bool {
        false
    }
    /// `checkForLostInputs()`.
    fn check_for_lost_inputs(&self) -> Result<(), ExecException> {
        Ok(())
    }
    /// `getClientEnv()`.
    fn get_client_env(&self) -> &IndexMap<String, String>;
    /// `getActionFileSystem()` when it is the `RemoteActionFileSystem` (see
    /// `ActionExecutionContext::get_remote_action_file_system`); `None` without one.
    // Java: com/google/devtools/build/lib/exec/SpawnRunner.java:269 SpawnRunner.SpawnExecutionContext.getActionFileSystem
    fn get_remote_action_file_system(
        &self,
    ) -> Option<Arc<crate::exec::remote_action_file_system::RemoteActionFileSystem>> {
        None
    }
    /// `getContext(RemoteLocalFallbackRegistry.class)`: the action's `SpawnStrategyRegistry`
    /// (registered as `RemoteLocalFallbackRegistry`), `None` where Java's `getContext` returns
    /// null.
    // Java: com/google/devtools/build/lib/exec/SpawnRunner.java:259 SpawnRunner.SpawnExecutionContext.getContext
    fn get_remote_local_fallback_registry(
        &self,
    ) -> Option<Arc<crate::exec::spawn_strategy_registry::SpawnStrategyRegistry>> {
        None
    }
}

/// A runner for spawns (`SpawnRunner`): local, sandboxed, remote, worker.
// Java: com/google/devtools/build/lib/exec/SpawnRunner.java:58 SpawnRunner
pub trait SpawnRunner: Send + Sync {
    /// `exec(Spawn, SpawnExecutionContext)`.
    fn exec(
        &self,
        spawn: &dyn Spawn,
        context: &dyn SpawnExecutionContext,
    ) -> Result<SpawnResult, ExecException>;

    /// `canExec(Spawn)`.
    fn can_exec(&self, spawn: &dyn Spawn) -> bool;

    /// `canExecWithLegacyFallback(Spawn)`.
    fn can_exec_with_legacy_fallback(&self, spawn: &dyn Spawn) -> bool {
        self.can_exec(spawn)
    }

    /// `handlesCaching()`.
    fn handles_caching(&self) -> bool;

    /// `getName()`.
    fn get_name(&self) -> String;

    /// `cleanupSandboxBase(Path, TreeDeleter)`.
    fn cleanup_sandbox_base(
        &self,
        _sandbox_base: &Path,
        _tree_deleter: &dyn TreeDeleter,
    ) -> std::io::Result<()> {
        Ok(())
    }

    /// `getSpawnResultBuilder(context)`.
    // Java: com/google/devtools/build/lib/exec/SpawnRunner.java:313 SpawnRunner.getSpawnResultBuilder
    fn get_spawn_result_builder(&self, context: &dyn SpawnExecutionContext) -> SpawnResultBuilder {
        let mut b = SpawnResultBuilder::new();
        b.set_runner_name(&self.get_name())
            .set_digest(context.get_digest());
        b
    }
}

impl fmt::Debug for dyn SpawnRunner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SpawnRunner({})", self.get_name())
    }
}
