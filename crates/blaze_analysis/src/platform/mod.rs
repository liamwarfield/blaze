//! Ports of `analysis/platform/*` (agent P; crates/blaze_analysis/DESIGN.md): the platform and
//! toolchain providers as frozen-capable Starlark values, and the analysis-capable
//! `platform_common` module.

pub mod constraint_collection;
pub mod constraint_constants;
pub mod constraint_setting_info;
pub mod constraint_value_info;
pub mod declared_toolchain_info;
pub mod platform_common;
pub mod platform_constants;
pub mod platform_info;
pub mod platform_properties;
pub mod platform_provider_utils;
pub mod platform_utils;
pub mod template_variable_info;
pub mod toolchain_info;
pub mod toolchain_type_info;

pub use constraint_collection::{
    ConstraintCollection, ConstraintCollectionBuilder, DuplicateConstraintException,
};
pub use constraint_constants::get_os_from_constraints_or_host;
pub use constraint_setting_info::ConstraintSettingInfo;
pub use constraint_value_info::ConstraintValueInfo;
pub use declared_toolchain_info::{
    DeclaredToolchainInfo, DeclaredToolchainInfoBuilder,
    DeclaredToolchainInfoDuplicateConstraintException,
};
pub use platform_common::register_platform_common;
pub use platform_info::{
    EMPTY_PLATFORM_INFO, ExecPropertiesException, PlatformInfo, PlatformInfoBuildError,
    PlatformInfoBuilder,
};
pub use platform_properties::{PlatformProperties, PlatformPropertiesBuilder};
pub use template_variable_info::TemplateVariableInfo;
pub use toolchain_info::{FrozenToolchainInfo, ToolchainInfo};
pub use toolchain_type_info::{ToolchainTypeInfo, toolchain_type_label_from_value};
