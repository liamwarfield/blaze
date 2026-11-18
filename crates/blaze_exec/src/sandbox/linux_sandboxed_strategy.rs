//! Port of `lib/sandbox/LinuxSandboxedStrategy.java`.

use std::sync::Arc;
use std::time::Duration;

use blaze_events::Event;
use blaze_optdefs::command::execution_options::ExecutionOptions;
use blaze_optdefs::misc::sandbox_options::SandboxOptions;
use blaze_vfs::Path;

use super::linux_sandbox_util;
use super::linux_sandboxed_spawn_runner::LinuxSandboxedSpawnRunner;
use crate::exec::abstract_spawn_strategy::AbstractSpawnStrategy;
use crate::exec::exec_command_environment::ExecCommandEnvironment;
use crate::exec::spawn_runner::SpawnRunner;
use crate::exec::tree_deleter::TreeDeleter;

/// Strategy that uses sandboxing to execute a process: an [`AbstractSpawnStrategy`] whose
/// `toString()` is `"linux-sandbox"`.
// Java: com/google/devtools/build/lib/sandbox/LinuxSandboxedStrategy.java:29 LinuxSandboxedStrategy
pub struct LinuxSandboxedStrategy;

impl LinuxSandboxedStrategy {
    /// `new LinuxSandboxedStrategy(spawnRunner, executionOptions)`.
    // Java: com/google/devtools/build/lib/sandbox/LinuxSandboxedStrategy.java:30 LinuxSandboxedStrategy.<init>
    #[allow(clippy::new_ret_no_self)]
    pub fn new(
        exec_root: Path,
        spawn_runner: Arc<dyn SpawnRunner>,
        execution_options: Arc<ExecutionOptions>,
    ) -> AbstractSpawnStrategy {
        // Java: com/google/devtools/build/lib/sandbox/LinuxSandboxedStrategy.java:35 LinuxSandboxedStrategy.toString
        AbstractSpawnStrategy::new(exec_root, spawn_runner, execution_options, "linux-sandbox")
            .with_simple_class_name("LinuxSandboxedStrategy")
    }

    /// Creates a sandboxed spawn runner that uses the `linux-sandbox` tool.
    ///
    /// `sandbox_base`: path to the sandbox base directory; `timeout_kill_delay`: additional grace
    /// period before killing timing out commands.
    // Java: com/google/devtools/build/lib/sandbox/LinuxSandboxedStrategy.java:46 LinuxSandboxedStrategy.create
    pub fn create(
        cmd_env: &ExecCommandEnvironment,
        sandbox_base: &Path,
        timeout_kill_delay: Duration,
        tree_deleter: Arc<dyn TreeDeleter>,
        options: &SandboxOptions,
    ) -> LinuxSandboxedSpawnRunner {
        let mut inaccessible_helper_file: Option<Path> = None;
        let mut inaccessible_helper_dir: Option<Path> = None;
        if !options.sandbox_block_path.is_empty() {
            let created: std::io::Result<()> = (|| {
                inaccessible_helper_file = Some(linux_sandbox_util::get_inaccessible_helper_file(
                    sandbox_base,
                )?);
                inaccessible_helper_dir = Some(linux_sandbox_util::get_inaccessible_helper_dir(
                    sandbox_base,
                )?);
                Ok(())
            })();
            if created.is_err() {
                cmd_env.reporter.handle(&Event::warn(format!(
                    "Could not block access to: {}",
                    options.sandbox_block_path.join(",")
                )));
            }
        }
        LinuxSandboxedSpawnRunner::new(
            cmd_env,
            sandbox_base.clone(),
            inaccessible_helper_file,
            inaccessible_helper_dir,
            timeout_kill_delay,
            tree_deleter,
        )
    }
}
