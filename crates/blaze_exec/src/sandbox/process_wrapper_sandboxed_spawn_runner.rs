//! Port of `lib/sandbox/ProcessWrapperSandboxedSpawnRunner.java`.

use std::io;
use std::sync::Arc;

use blaze_vfs::{Path, PathFragment};

use super::abstract_sandbox_spawn_runner::{
    AbstractSandboxSpawnRunner, SandboxRunnerError, SandboxSpawnRunnerHooks,
};
use super::sandbox_helpers;
use super::sandboxed_spawn::SandboxedSpawn;
use super::symlinked_sandboxed_spawn::SymlinkedSandboxedSpawn;
use crate::actions::{ExecException, Spawn, SpawnResult};
use crate::exec::exec_command_environment::ExecCommandEnvironment;
use crate::exec::local::local_env_provider::{self, LocalEnvProvider, PosixLocalEnvProvider};
use crate::exec::spawn_runner::{SpawnExecutionContext, SpawnRunner};
use crate::exec::tree_deleter::TreeDeleter;
use crate::runtime::process_wrapper::ProcessWrapper;

/// `ProcessWrapper.fromCommandEnvironment(cmdEnv)`.
// Java: com/google/devtools/build/lib/runtime/ProcessWrapper.java:66 ProcessWrapper.fromCommandEnvironment
fn process_wrapper_from_command_environment(
    cmd_env: &ExecCommandEnvironment,
) -> Option<ProcessWrapper> {
    let options = &cmd_env.local_execution_options;
    ProcessWrapper::from_bin_tools(
        &cmd_env.bin_tools,
        Some(options.get_local_sigkill_grace_seconds()),
        options.process_wrapper_graceful_sigterm,
    )
}

/// Strategy that uses sandboxing to execute a process.
// Java: com/google/devtools/build/lib/sandbox/ProcessWrapperSandboxedSpawnRunner.java:32 ProcessWrapperSandboxedSpawnRunner
pub struct ProcessWrapperSandboxedSpawnRunner {
    base: AbstractSandboxSpawnRunner,
    process_wrapper: ProcessWrapper,
    exec_root: Path,
    sandbox_base: Path,
    local_env_provider: PosixLocalEnvProvider,
    tree_deleter: Arc<dyn TreeDeleter>,
}

impl ProcessWrapperSandboxedSpawnRunner {
    /// `isSupported(cmdEnv)`.
    // Java: com/google/devtools/build/lib/sandbox/ProcessWrapperSandboxedSpawnRunner.java:34 ProcessWrapperSandboxedSpawnRunner.isSupported
    pub fn is_supported(cmd_env: &ExecCommandEnvironment) -> bool {
        // PORT: OS.isPosixCompatible() is true on Linux.
        process_wrapper_from_command_environment(cmd_env).is_some()
    }

    /// Creates a sandboxed spawn runner that uses the `process-wrapper` tool.
    ///
    /// `sandbox_base`: path to the sandbox base directory.
    // Java: com/google/devtools/build/lib/sandbox/ProcessWrapperSandboxedSpawnRunner.java:50 ProcessWrapperSandboxedSpawnRunner.<init>
    pub fn new(
        cmd_env: &ExecCommandEnvironment,
        sandbox_base: Path,
        tree_deleter: Arc<dyn TreeDeleter>,
    ) -> ProcessWrapperSandboxedSpawnRunner {
        ProcessWrapperSandboxedSpawnRunner {
            base: AbstractSandboxSpawnRunner::new(cmd_env),
            process_wrapper: process_wrapper_from_command_environment(cmd_env)
                .expect("NullPointerException: process-wrapper is not an embedded tool"),
            exec_root: cmd_env.exec_root.clone(),
            local_env_provider: local_env_provider::for_current_os(&cmd_env.client_env),
            sandbox_base,
            tree_deleter,
        }
    }

    /// `prepareSpawn(spawn, context)`.
    // Java: com/google/devtools/build/lib/sandbox/ProcessWrapperSandboxedSpawnRunner.java:61 ProcessWrapperSandboxedSpawnRunner.prepareSpawn
    pub fn prepare_spawn(
        &self,
        spawn: &dyn Spawn,
        context: &dyn SpawnExecutionContext,
    ) -> Result<Box<dyn SandboxedSpawn>, SandboxRunnerError> {
        // Each invocation of "exec" gets its own sandbox base.
        // Note that the value returned by context.getId() is only unique inside one given
        // SpawnRunner, so we have to prefix our name to turn it into a globally unique value.
        let sandbox_path = self
            .sandbox_base
            .get_relative_str(&self.get_name())
            .get_relative_str(&context.get_id().to_string());
        sandbox_path.create_directory_and_parents()?;

        // b/64689608: The execroot of the sandboxed process must end with the workspace name, just
        // like the normal execroot does.
        let workspace_name = self.exec_root.get_base_name();
        let sandbox_exec_root = sandbox_path
            .get_relative_str("execroot")
            .get_relative_str(workspace_name);
        sandbox_exec_root.create_directory_and_parents()?;

        let environment = self.local_env_provider.rewrite_local_env(
            spawn.get_environment(),
            &self.base.bin_tools,
            "/tmp",
        )?;

        let timeout = context.get_timeout();
        let mut command_line_builder = self
            .process_wrapper
            .command_line_builder(spawn.get_arguments().to_vec());
        command_line_builder
            .add_execution_info(spawn.get_execution_info())
            .set_timeout(timeout);

        let statistics_path = sandbox_path.get_relative_str("stats.out");
        command_line_builder.set_statistics_path(statistics_path.as_fragment().clone());

        let inputs = sandbox_helpers::process_input_files(
            &context.get_input_mapping(
                &PathFragment::empty_fragment(),
                /* will_access_repeatedly= */ true,
            ),
            &self.exec_root,
        )?;
        let outputs = sandbox_helpers::get_outputs(spawn);

        let writable_dirs = self
            .base
            .get_writable_dirs(&sandbox_exec_root, &environment)?;
        Ok(Box::new(SymlinkedSandboxedSpawn::new(
            sandbox_path,
            sandbox_exec_root,
            command_line_builder.build(),
            environment,
            inputs,
            outputs,
            writable_dirs,
            self.tree_deleter.clone(),
            /* sandbox_debug_path= */ None,
            Some(statistics_path),
            /* interactive_debug_arguments= */ None,
            &spawn.get_mnemonic(),
            spawn.get_target_label(),
        )))
    }
}

impl SandboxSpawnRunnerHooks for ProcessWrapperSandboxedSpawnRunner {
    fn runner_name(&self) -> String {
        self.get_name()
    }

    fn prepare_spawn(
        &self,
        spawn: &dyn Spawn,
        context: &dyn SpawnExecutionContext,
    ) -> Result<Box<dyn SandboxedSpawn>, SandboxRunnerError> {
        ProcessWrapperSandboxedSpawnRunner::prepare_spawn(self, spawn, context)
    }
}

impl SpawnRunner for ProcessWrapperSandboxedSpawnRunner {
    fn exec(
        &self,
        spawn: &dyn Spawn,
        context: &dyn SpawnExecutionContext,
    ) -> Result<SpawnResult, ExecException> {
        self.base.exec(self, spawn, context)
    }

    fn can_exec(&self, spawn: &dyn Spawn) -> bool {
        AbstractSandboxSpawnRunner::can_exec(spawn)
    }

    fn handles_caching(&self) -> bool {
        false
    }

    // Java: com/google/devtools/build/lib/sandbox/ProcessWrapperSandboxedSpawnRunner.java:112 ProcessWrapperSandboxedSpawnRunner.getName
    fn get_name(&self) -> String {
        "processwrapper-sandbox".to_string()
    }

    fn cleanup_sandbox_base(
        &self,
        sandbox_base: &Path,
        tree_deleter: &dyn TreeDeleter,
    ) -> io::Result<()> {
        self.base
            .cleanup_sandbox_base(&self.get_name(), sandbox_base, tree_deleter)
    }
}
