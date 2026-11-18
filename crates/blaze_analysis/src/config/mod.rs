//! Ports of `analysis/config` (attribute mapping of `select()`, config conditions, transitions)
//! and the configuration-related providers analysis needs (agent C;
//! crates/blaze_analysis/DESIGN.md).

pub mod build_option_details;
pub mod build_setting_provider;
pub mod config_conditions;
pub mod config_matching_provider;
pub mod configured_attribute_mapper;
pub mod incompatible_platform_provider;
pub mod ordered_set_multimap;
pub mod starlark_transition;
pub mod transitions;
