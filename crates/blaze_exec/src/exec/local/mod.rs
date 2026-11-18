//! Port of `lib/exec/local`.

pub mod local_env_provider;
pub mod local_spawn_runner;

pub use local_env_provider::{LocalEnvProvider, NoopLocalEnvProvider, PosixLocalEnvProvider};
