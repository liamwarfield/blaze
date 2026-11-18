//! Toolchain resolution SkyFunctions (agent P; crates/blaze_analysis/DESIGN.md): ports of
//! `skyframe/toolchains/*` functions and helpers, `skyframe/TargetPatternUtil` and
//! `analysis/producers/UnloadedToolchainContextsProducer`.

use std::sync::Arc;

use blaze_analysis::analysis_services::AnalysisServices;
use blaze_analysis::toolchains::ToolchainContextKey;
use blaze_skyframe::EvaluatorBuilder;

pub mod config_matching_util;
pub mod constraint_value_lookup_util;
pub mod lookup_error;
pub mod platform_keys;
pub mod platform_lookup_util;
pub mod registered_execution_platforms;
pub mod registered_toolchains;
pub mod single_toolchain_resolution;
pub mod target_pattern_util;
pub mod toolchain_context_util;
pub mod toolchain_resolution_function;
pub mod toolchain_type_lookup_util;
pub mod unloaded_toolchain_contexts_producer;

pub use blaze_analysis::toolchains::UnloadedToolchainContextsInputs;

#[cfg(test)]
mod tests;
pub use registered_execution_platforms::{
    RegisteredExecutionPlatformsKey, RegisteredExecutionPlatformsValue,
};
pub use registered_toolchains::{RegisteredToolchainsKey, RegisteredToolchainsValue};
pub use single_toolchain_resolution::{
    SingleToolchainResolutionKey, SingleToolchainResolutionValue,
};

/// Registers REGISTERED_EXECUTION_PLATFORMS, REGISTERED_TOOLCHAINS, SINGLE_TOOLCHAIN_RESOLUTION
/// and TOOLCHAIN_RESOLUTION (Java: `SkyframeExecutor.skyFunctions()`).
///
/// PLATFORM is registered by the configuration functions through
/// `blaze_build_config::platform::PlatformValueProvider`, implemented by
/// `crate::platform::platform_function::PlatformFunction`.
pub fn register_toolchain_functions(
    builder: EvaluatorBuilder,
    _services: Arc<AnalysisServices>,
) -> EvaluatorBuilder {
    builder
        .typed_function::<RegisteredExecutionPlatformsKey, _>(
            registered_execution_platforms::registered_execution_platforms_function,
        )
        .typed_function::<RegisteredToolchainsKey, _>(
            registered_toolchains::registered_toolchains_function,
        )
        .typed_function::<SingleToolchainResolutionKey, _>(
            single_toolchain_resolution::single_toolchain_resolution_function,
        )
        .typed_function::<ToolchainContextKey, _>(
            toolchain_resolution_function::toolchain_resolution_function,
        )
}
