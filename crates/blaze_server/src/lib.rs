//! Port of `com.google.devtools.build.lib.server` (decision D3: a crate below `blaze_runtime`),
//! plus the parts of `lib/runtime` the server needs (`CommandDispatcher`,
//! `BlazeCommandResult`) and `lib/unix/ProcMeminfoParser`.
//!
//! Not ported here (later package S4): `GcAndInternerShrinkingIdleTask`,
//! `InstallBaseGarbageCollector` and `InstallBaseGarbageCollectorIdleTask`.
#![allow(clippy::too_many_arguments, clippy::new_without_default)]

pub mod blaze_command_result;
pub mod command_dispatcher;
pub mod command_manager;
pub mod grpc;
pub mod grpc_server_impl;
pub mod idle_task;
pub mod idle_task_manager;
pub mod interruptible;
pub mod pid_file_watcher;
pub mod proc_meminfo_parser;
pub mod server_watcher_runnable;
pub mod shutdown_hooks;
pub mod terminal_size_monitor;
pub mod uncaught_exception;

pub use blaze_command_result::BlazeCommandResult;
pub use command_dispatcher::{
    CommandDispatcher, CommandExtensionReporter, IdleTaskResultsSupplier, LockingMode, UiVerbosity,
};
pub use grpc_server_impl::{AbruptExitException, GrpcServerImpl};
pub use idle_task::{IdleTask, IdleTaskError, IdleTaskException, IdleTaskResult, IdleTaskStatus};
pub use pid_file_watcher::PidFileWatcher;
pub use shutdown_hooks::ShutdownHooks;
pub use terminal_size_monitor::TerminalSizeMonitor;
