//! The subset of `runtime/CommandEnvironment` (and `BlazeWorkspace`/`BlazeRuntime`) that spawn
//! runners and strategies read. `blaze_runtime` builds one per command.

use std::sync::Arc;

use blaze_events::EventHandler;
use blaze_optdefs::command::execution_options::ExecutionOptions;
use blaze_optdefs::misc::local_execution_options::LocalExecutionOptions;
use blaze_optdefs::misc::sandbox_options::SandboxOptions;
use blaze_vfs::{FileSystemRef, Path};
use indexmap::IndexMap;

use crate::actions::resource_manager::ResourceManager;
use crate::exec::bin_tools::BinTools;

/// What `CommandEnvironment` gives the execution layer.
// Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java CommandEnvironment
#[derive(Clone)]
pub struct ExecCommandEnvironment {
    /// `getRuntime().getFileSystem()`.
    pub file_system: FileSystemRef,
    /// `getExecRoot()`: `<output_base>/execroot/<workspace name>`.
    pub exec_root: Path,
    /// `getOutputBase()`.
    pub output_base: Path,
    /// `getClientEnv()`.
    pub client_env: IndexMap<String, String>,
    /// `getBlazeWorkspace().getBinTools()`.
    pub bin_tools: Arc<BinTools>,
    /// `getOptions().getOptions(SandboxOptions.class)`.
    pub sandbox_options: Arc<SandboxOptions>,
    /// `getOptions().getOptions(ExecutionOptions.class)`.
    pub execution_options: Arc<ExecutionOptions>,
    /// `getOptions().getOptions(LocalExecutionOptions.class)`.
    pub local_execution_options: Arc<LocalExecutionOptions>,
    /// `getOptions().getOptions(RemoteOptions.class)`.
    pub remote_options: Arc<blaze_optdefs::misc::remote_options::RemoteOptions>,
    /// `getOptions().getOptions(DynamicExecutionOptions.class)`.
    pub dynamic_execution_options:
        Arc<blaze_optdefs::misc::dynamic_execution_options::DynamicExecutionOptions>,
    /// `getOptions().getOptions(WorkerOptions.class)`.
    pub worker_options: Arc<blaze_optdefs::misc::worker_options::WorkerOptions>,
    /// `getLocalResourceManager()`.
    pub resource_manager: Arc<ResourceManager>,
    /// `getReporter()`.
    pub reporter: Arc<dyn EventHandler>,
    /// `getPackageLocator().getPathEntries()` (as paths).
    pub package_path_entries: Vec<Path>,
    /// `getRepoContentsCachePath()`.
    pub repo_contents_cache_path: Option<Path>,
    /// `getOptions(TestConfiguration.TestOptions.class).testArguments` (empty for non-test
    /// commands).
    pub test_arguments: Vec<String>,
    /// `getOptions().getOptions(BuildRequestOptions.class).jobs` (read by the dynamic strategy;
    /// `None` for tools that never register it).
    pub jobs: Option<i32>,
    /// `getCommandId().toString()` (the Docker sandbox labels its containers with it; empty for
    /// tools without a command).
    pub command_id: String,
}
