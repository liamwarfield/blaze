//! Port of `lib/dynamic/`: dynamic execution (`--spawn_strategy=dynamic`), which runs a spawn
//! locally and remotely at once and keeps the result of the branch that finishes first.

pub mod branch;
pub mod dynamic_execution_module;
pub mod dynamic_spawn_strategy;
pub mod shrinkable_semaphore;
