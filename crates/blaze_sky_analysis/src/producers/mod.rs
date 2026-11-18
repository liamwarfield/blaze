//! Ports of `analysis/producers/*` (except `PlatformProducer` and
//! `UnloadedToolchainContexts*`, agent P) as straight-line functions over the blocking
//! evaluator, plus the skyframe helpers they share (agent C; crates/blaze_analysis/DESIGN.md).

pub mod config_conditions_producer;
pub mod configured_target_and_data_producer;
pub mod dependency_context_producer;
pub mod dependency_map_producer;
pub mod dependency_resolution_helpers;
pub mod errors;
pub mod starlark_build_settings_details_function;
pub mod target_and_configuration_producer;
pub mod transition_applier;
