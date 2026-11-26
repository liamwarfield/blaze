//! Port of `lib/sandbox` (Linux parts).
//!
//! Not ported: Darwin, Windows and Docker runners (`unported!` where a flag asks for them),
//! `CopyingSandboxedSpawn`, and cgroups (`CgroupsInfo` for `--experimental_sandbox_memory_limit_mb`
//! and `VirtualCgroupFactory` for `--experimental_sandbox_use_new_cgroup_implementation` are
//! `unported!` at their use sites).

pub mod abstract_containerizing_sandboxed_spawn;
pub mod abstract_sandbox_spawn_runner;
pub mod asynchronous_tree_deleter;
pub mod cgroups;
mod fs_utils;
pub mod hardlinked_sandboxed_spawn;
pub mod linux_sandbox_command_line_builder;
pub mod linux_sandbox_util;
pub mod linux_sandboxed_spawn_runner;
pub mod linux_sandboxed_strategy;
pub mod process_wrapper_sandboxed_spawn_runner;
pub mod process_wrapper_sandboxed_strategy;
pub mod sandbox_helpers;
pub mod sandbox_module;
pub mod sandbox_stash;
pub mod sandboxed_spawn;
pub mod symlinked_sandboxed_spawn;
pub mod synchronous_tree_deleter;

pub use abstract_containerizing_sandboxed_spawn::{
    AbstractContainerizingSandboxedSpawn, ContainerizingSandboxedSpawnHooks,
};
pub use abstract_sandbox_spawn_runner::{
    AbstractSandboxSpawnRunner, SANDBOX_DEBUG_SUGGESTION, SandboxRunnerError,
    SandboxSpawnRunnerHooks,
};
pub use asynchronous_tree_deleter::AsynchronousTreeDeleter;
pub use hardlinked_sandboxed_spawn::HardlinkedSandboxedSpawn;
pub use linux_sandbox_command_line_builder::{LinuxSandboxCommandLineBuilder, NetworkNamespace};
pub use linux_sandboxed_spawn_runner::LinuxSandboxedSpawnRunner;
pub use linux_sandboxed_strategy::LinuxSandboxedStrategy;
pub use process_wrapper_sandboxed_spawn_runner::ProcessWrapperSandboxedSpawnRunner;
pub use process_wrapper_sandboxed_strategy::ProcessWrapperSandboxedStrategy;
pub use sandbox_helpers::{SandboxContents, SandboxInputs, SandboxOutputs};
pub use sandbox_module::SandboxModule;
pub use sandbox_stash::SandboxStash;
pub use sandboxed_spawn::SandboxedSpawn;
pub use symlinked_sandboxed_spawn::SymlinkedSandboxedSpawn;
pub use synchronous_tree_deleter::SynchronousTreeDeleter;
