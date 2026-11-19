//! Toolchain context types (agent P; crates/blaze_analysis/DESIGN.md): ports of
//! `skyframe/toolchains/{ToolchainContextKey, UnloadedToolchainContext(Impl), ToolchainException,
//! NoMatchingPlatformData, NoMatchingPlatformException}` and `analysis/{ToolchainContext,
//! ToolchainCollection, ResolvedToolchainContext, ExecGroupCollection}` and
//! `analysis/producers/UnloadedToolchainContextsInputs`.
//!
//! `ToolchainTypeRequirement` is `blaze_packages::rule_class::ToolchainTypeRequirement`.

pub mod aspect_base_target_resolved_toolchain_context;
pub mod exec_group_collection;
pub mod no_matching_platform_data;
pub mod resolved_toolchain_context;
pub mod toolchain_collection;
pub mod toolchain_context;
pub mod toolchain_context_key;
pub mod toolchain_exception;
pub mod unloaded_toolchain_context;
pub mod unloaded_toolchain_contexts_inputs;

pub use blaze_packages::rule_class::ToolchainTypeRequirement;
pub use exec_group_collection::{
    ExecGroupCollection, ExecGroupCollectionBuilder, InvalidExecGroupException,
};
pub use no_matching_platform_data::{NoMatchingPlatformData, no_matching_platform_exception};
pub use resolved_toolchain_context::ResolvedToolchainContext;
pub use toolchain_collection::{ToolchainCollection, ToolchainCollectionBuilder};
pub use toolchain_context::ToolchainContext;
pub use toolchain_context_key::{
    TOOLCHAIN_RESOLUTION, ToolchainContextKey, ToolchainContextKeyBuilder,
};
pub use toolchain_exception::{ToolchainException, ToolchainExceptionKind};
pub use unloaded_toolchain_context::{
    UnloadedToolchainContext, UnloadedToolchainContextImpl, UnloadedToolchainContextImplBuilder,
};
pub use unloaded_toolchain_contexts_inputs::UnloadedToolchainContextsInputs;

#[cfg(test)]
mod tests;
