//! Port of `lib/standalone/StandaloneSpawnStrategy.java`.

use std::sync::Arc;

use blaze_optdefs::command::execution_options::ExecutionOptions;
use blaze_vfs::Path;

use crate::exec::abstract_spawn_strategy::AbstractSpawnStrategy;
use crate::exec::spawn_runner::SpawnRunner;

/// Strategy that uses subprocessing to execute a process.
///
/// An [`AbstractSpawnStrategy`] whose `toString()` is `"standalone"`.
// Java: com/google/devtools/build/lib/standalone/StandaloneSpawnStrategy.java:23 StandaloneSpawnStrategy
pub struct StandaloneSpawnStrategy;

impl StandaloneSpawnStrategy {
    /// `new StandaloneSpawnStrategy(spawnRunner, executionOptions)`.
    // Java: com/google/devtools/build/lib/standalone/StandaloneSpawnStrategy.java:24 StandaloneSpawnStrategy.<init>
    #[allow(clippy::new_ret_no_self)]
    pub fn new(
        exec_root: Path,
        spawn_runner: Arc<dyn SpawnRunner>,
        execution_options: Arc<ExecutionOptions>,
    ) -> AbstractSpawnStrategy {
        // Java: com/google/devtools/build/lib/standalone/StandaloneSpawnStrategy.java:29 StandaloneSpawnStrategy.toString
        AbstractSpawnStrategy::new(exec_root, spawn_runner, execution_options, "standalone")
            .with_simple_class_name("StandaloneSpawnStrategy")
    }
}
