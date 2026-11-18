//! Port of `lib/standalone`: the standalone (local, unsandboxed) spawn strategy and its module.

pub mod standalone_module;
pub mod standalone_spawn_strategy;

pub use standalone_module::StandaloneModule;
pub use standalone_spawn_strategy::StandaloneSpawnStrategy;
