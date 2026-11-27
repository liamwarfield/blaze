//! Ports of `skyframe/toolchains/NoMatchingPlatformData.java` and
//! `NoMatchingPlatformException.java`.

use blaze_packages::rule_class::ToolchainTypeRequirement;

use crate::configured_target_key::ConfiguredTargetKey;
use crate::toolchains::toolchain_exception::{ToolchainException, ToolchainExceptionKind};

/// Contains information related to missing execution platform.
// Java: com/google/devtools/build/lib/skyframe/toolchains/NoMatchingPlatformData.java:27 NoMatchingPlatformData
// PORT: skipped, the AutoValue builder is a struct literal at its callers: NoMatchingPlatformData.builder, NoMatchingPlatformData.Builder.setToolchainTypes, NoMatchingPlatformData.Builder.setAvailableExecutionPlatformKeys, NoMatchingPlatformData.Builder.setTargetPlatformKey, NoMatchingPlatformData.Builder.build
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoMatchingPlatformData {
    pub toolchain_types: Vec<ToolchainTypeRequirement>,
    pub available_execution_platform_keys: Vec<ConfiguredTargetKey>,
    pub target_platform_key: ConfiguredTargetKey,
}

impl NoMatchingPlatformData {
    // Java: com/google/devtools/build/lib/skyframe/toolchains/NoMatchingPlatformData.java:50 NoMatchingPlatformData.formatError
    pub fn format_error(&self) -> String {
        let platforms = self
            .available_execution_platform_keys
            .iter()
            .map(|key| key.get_label().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        if self.toolchain_types.is_empty() {
            return format!(
                "Unable to find an execution platform for target platform {} from available execution platforms [{}]",
                self.target_platform_key.get_label(),
                platforms
            );
        }
        format!(
            "Unable to find an execution platform for toolchains [{}] and target platform {} from available execution platforms [{}]",
            self.toolchain_types
                .iter()
                .map(|t| t.toolchain_type.to_string())
                .collect::<Vec<_>>()
                .join(", "),
            self.target_platform_key.get_label(),
            platforms
        )
    }
}

/// Indicates a missing execution platform (Java `NoMatchingPlatformException`).
// Java: com/google/devtools/build/lib/skyframe/toolchains/NoMatchingPlatformException.java:20 NoMatchingPlatformException.<init>
pub fn no_matching_platform_exception(error: &NoMatchingPlatformData) -> ToolchainException {
    ToolchainException::new(
        ToolchainExceptionKind::NoMatchingPlatform,
        error.format_error(),
    )
}
