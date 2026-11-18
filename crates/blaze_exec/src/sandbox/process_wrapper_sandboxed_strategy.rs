//! Port of `lib/sandbox/ProcessWrapperSandboxedStrategy.java`.

use std::sync::Arc;

use blaze_optdefs::command::execution_options::ExecutionOptions;
use blaze_vfs::Path;

use crate::exec::abstract_spawn_strategy::AbstractSpawnStrategy;
use crate::exec::spawn_runner::SpawnRunner;

/// Strategy that uses sandboxing to execute a process: an [`AbstractSpawnStrategy`] whose
/// `toString()` is `"processwrapper-sandbox"`.
// Java: com/google/devtools/build/lib/sandbox/ProcessWrapperSandboxedStrategy.java:22 ProcessWrapperSandboxedStrategy
pub struct ProcessWrapperSandboxedStrategy;

impl ProcessWrapperSandboxedStrategy {
    /// `new ProcessWrapperSandboxedStrategy(spawnRunner, executionOptions)`.
    // Java: com/google/devtools/build/lib/sandbox/ProcessWrapperSandboxedStrategy.java:23 ProcessWrapperSandboxedStrategy.<init>
    #[allow(clippy::new_ret_no_self)]
    pub fn new(
        exec_root: Path,
        spawn_runner: Arc<dyn SpawnRunner>,
        execution_options: Arc<ExecutionOptions>,
    ) -> AbstractSpawnStrategy {
        // Java: com/google/devtools/build/lib/sandbox/ProcessWrapperSandboxedStrategy.java:28 ProcessWrapperSandboxedStrategy.toString
        AbstractSpawnStrategy::new(
            exec_root,
            spawn_runner,
            execution_options,
            "processwrapper-sandbox",
        )
        .with_simple_class_name("ProcessWrapperSandboxedStrategy")
    }
}
