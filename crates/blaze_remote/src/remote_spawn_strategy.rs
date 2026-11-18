//! Port of `lib/remote/RemoteSpawnStrategy.java`.

use std::sync::Arc;

use blaze_exec::exec::SpawnRunner;
use blaze_exec::exec::abstract_spawn_strategy::AbstractSpawnStrategy;
use blaze_optdefs::command::execution_options::ExecutionOptions;
use blaze_vfs::Path;

/// Strategy that uses a distributed cache for sharing action input and output files. Optionally
/// this strategy also support offloading the work to a remote worker.
///
/// `new RemoteSpawnStrategy(spawnRunner, executionOptions)`: an `AbstractSpawnStrategy` whose
/// `toString()` is `"remote"`.
///
/// PORT: Java subclasses of `AbstractSpawnStrategy` only override `toString()`; here they are an
/// `AbstractSpawnStrategy` with that display name (see `blaze_exec`). `exec_root` is the extra
/// argument of `AbstractSpawnStrategy::new`.
// Java: com/google/devtools/build/lib/remote/RemoteSpawnStrategy.java:25 RemoteSpawnStrategy
pub fn new_remote_spawn_strategy(
    exec_root: Path,
    spawn_runner: Arc<dyn SpawnRunner>,
    execution_options: Arc<ExecutionOptions>,
) -> AbstractSpawnStrategy {
    // Java: com/google/devtools/build/lib/remote/RemoteSpawnStrategy.java:26 RemoteSpawnStrategy.<init>
    // Java: com/google/devtools/build/lib/remote/RemoteSpawnStrategy.java:31 RemoteSpawnStrategy.toString
    AbstractSpawnStrategy::new(exec_root, spawn_runner, execution_options, "remote")
        .with_simple_class_name("RemoteSpawnStrategy")
}
