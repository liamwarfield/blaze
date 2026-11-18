//! Port of `lib/exec` (+ `lib/exec/local`).

pub mod abstract_spawn_strategy;
pub mod action_execution_context;
pub mod bin_tools;
pub mod compact_spawn_log_context;
pub mod exec_command_environment;
pub mod executor_setup;
pub mod expanded_spawn_log_context;
pub mod io;
pub mod local;
pub mod spawn_input_expander;
pub mod spawn_log_context;
pub mod spawn_log_module;
pub mod spawn_log_reconstructor;
pub mod spawn_runner;
pub mod spawn_strategy_policy;
pub mod spawn_strategy_registry;
pub mod spawn_strategy_resolver;
pub mod stable_sort;
pub mod tree_deleter;

pub use bin_tools::BinTools;
pub use exec_command_environment::ExecCommandEnvironment;
pub use spawn_log_context::{SpawnLogAction, SpawnLogContext};
pub use spawn_runner::{ProgressStatus, SpawnExecutionContext, SpawnRunner};
pub use tree_deleter::TreeDeleter;
