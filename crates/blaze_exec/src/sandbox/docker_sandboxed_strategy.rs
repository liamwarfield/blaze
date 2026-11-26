//! Port of `lib/sandbox/DockerSandboxedStrategy.java`.

use std::sync::Arc;

use blaze_optdefs::command::execution_options::ExecutionOptions;
use blaze_vfs::Path;

use crate::exec::abstract_spawn_strategy::AbstractSpawnStrategy;
use crate::exec::spawn_runner::SpawnRunner;

/// Strategy that uses Docker to execute a process: an [`AbstractSpawnStrategy`] whose
/// `toString()` is `"docker"`.
// Java: com/google/devtools/build/lib/sandbox/DockerSandboxedStrategy.java:21 DockerSandboxedStrategy
pub struct DockerSandboxedStrategy;

impl DockerSandboxedStrategy {
    // Java: com/google/devtools/build/lib/sandbox/DockerSandboxedStrategy.java:22 DockerSandboxedStrategy.<init>
    #[allow(clippy::new_ret_no_self)]
    pub fn new(
        exec_root: Path,
        spawn_runner: Arc<dyn SpawnRunner>,
        execution_options: Arc<ExecutionOptions>,
    ) -> AbstractSpawnStrategy {
        // Java: com/google/devtools/build/lib/sandbox/DockerSandboxedStrategy.java:27 DockerSandboxedStrategy.toString
        AbstractSpawnStrategy::new(exec_root, spawn_runner, execution_options, "docker")
            .with_simple_class_name("DockerSandboxedStrategy")
    }
}
