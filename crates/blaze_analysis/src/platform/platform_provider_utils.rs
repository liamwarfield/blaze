//! Port of `analysis/platform/PlatformProviderUtils.java`: utility methods to help locate
//! platform-related providers.
//!
//! PORT: Java takes any `ProviderCollection`; the Rust port takes configured targets (the
//! provider collections these helpers are used with).

use starlark::values::OwnedFrozenValue;

use crate::configured_target::ConfiguredTarget;
use crate::info::downcast_info;
use crate::platform::constraint_setting_info::ConstraintSettingInfo;
use crate::platform::constraint_value_info::ConstraintValueInfo;
use crate::platform::declared_toolchain_info::DeclaredToolchainInfo;
use crate::platform::platform_info::PlatformInfo;
use crate::platform::toolchain_info::FrozenToolchainInfo;
use crate::platform::toolchain_type_info::ToolchainTypeInfo;
use blaze_packages::starlark_defs::platform_common::{
    CONSTRAINT_SETTING_INFO, CONSTRAINT_VALUE_INFO, PLATFORM_INFO, TOOLCHAIN_INFO,
    TOOLCHAIN_TYPE_INFO,
};

/// Retrieves and casts the [`PlatformInfo`] provider from the given target.
// Java: com/google/devtools/build/lib/analysis/platform/PlatformProviderUtils.java:30 PlatformProviderUtils.platform
pub fn platform(target: Option<&ConfiguredTarget>) -> Option<PlatformInfo> {
    let target = target?;
    target
        .get(&PLATFORM_INFO.key())
        .as_ref()
        .and_then(|v| downcast_info::<PlatformInfo>(v))
        .cloned()
}

/// Retrieves and casts [`PlatformInfo`] providers from the given targets.
// Java: com/google/devtools/build/lib/analysis/platform/PlatformProviderUtils.java:38 PlatformProviderUtils.platforms
pub fn platforms<'a>(targets: impl IntoIterator<Item = &'a ConfiguredTarget>) -> Vec<PlatformInfo> {
    targets
        .into_iter()
        .filter_map(|t| platform(Some(t)))
        .collect()
}

/// Retrieves and casts the [`ConstraintSettingInfo`] provider from the given target.
// Java: com/google/devtools/build/lib/analysis/platform/PlatformProviderUtils.java:47 PlatformProviderUtils.constraintSetting
pub fn constraint_setting(target: Option<&ConfiguredTarget>) -> Option<ConstraintSettingInfo> {
    let target = target?;
    target
        .get(&CONSTRAINT_SETTING_INFO.key())
        .as_ref()
        .and_then(|v| downcast_info::<ConstraintSettingInfo>(v))
        .cloned()
}

/// Retrieves and casts the [`ConstraintValueInfo`] provider from the given target.
// Java: com/google/devtools/build/lib/analysis/platform/PlatformProviderUtils.java:56 PlatformProviderUtils.constraintValue
pub fn constraint_value(target: Option<&ConfiguredTarget>) -> Option<ConstraintValueInfo> {
    let target = target?;
    target
        .get(&CONSTRAINT_VALUE_INFO.key())
        .as_ref()
        .and_then(|v| downcast_info::<ConstraintValueInfo>(v))
        .cloned()
}

/// Returns if a target provides [`ConstraintValueInfo`].
// Java: com/google/devtools/build/lib/analysis/platform/PlatformProviderUtils.java:64 PlatformProviderUtils.hasConstraintValue
pub fn has_constraint_value(target: &ConfiguredTarget) -> bool {
    target.get(&CONSTRAINT_VALUE_INFO.key()).is_some()
}

/// Retrieves and casts [`ConstraintValueInfo`] providers from the given targets.
// Java: com/google/devtools/build/lib/analysis/platform/PlatformProviderUtils.java:69 PlatformProviderUtils.constraintValues
pub fn constraint_values<'a>(
    targets: impl IntoIterator<Item = &'a ConfiguredTarget>,
) -> Vec<ConstraintValueInfo> {
    targets
        .into_iter()
        .filter_map(|t| constraint_value(Some(t)))
        .collect()
}

/// Retrieves and casts the [`DeclaredToolchainInfo`] from a `toolchain` rule. Returns `None`
/// if the rule isn't a toolchain.
// Java: com/google/devtools/build/lib/analysis/platform/PlatformProviderUtils.java:84 PlatformProviderUtils.declaredToolchainInfo
pub fn declared_toolchain_info(target: Option<&ConfiguredTarget>) -> Option<DeclaredToolchainInfo> {
    let target = target?;
    target.get_provider::<DeclaredToolchainInfo>().cloned()
}

/// Retrieves the `ToolchainInfo` provider from the given target (a frozen
/// [`FrozenToolchainInfo`]).
// Java: com/google/devtools/build/lib/analysis/platform/PlatformProviderUtils.java:93 PlatformProviderUtils.toolchain
pub fn toolchain(target: Option<&ConfiguredTarget>) -> Option<OwnedFrozenValue> {
    let target = target?;
    let v = target.get(&TOOLCHAIN_INFO.key())?;
    // Java: target.get(ToolchainInfo.PROVIDER) returns the value of the provider's class.
    downcast_info::<FrozenToolchainInfo>(&v)?;
    Some(v)
}

/// Retrieves and casts the [`ToolchainTypeInfo`] provider from the given target.
// Java: com/google/devtools/build/lib/analysis/platform/PlatformProviderUtils.java:101 PlatformProviderUtils.toolchainType
pub fn toolchain_type(target: &ConfiguredTarget) -> Option<ToolchainTypeInfo> {
    target
        .get(&TOOLCHAIN_TYPE_INFO.key())
        .as_ref()
        .and_then(|v| downcast_info::<ToolchainTypeInfo>(v))
        .cloned()
}
