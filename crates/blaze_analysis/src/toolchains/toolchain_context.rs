//! Port of `analysis/ToolchainContext.java`.

use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_packages::rule_class::ToolchainTypeRequirement;

use crate::platform::PlatformInfo;
use crate::toolchains::toolchain_context_key::ToolchainContextKey;

/// Represents the data needed for a specific target's use of toolchains and platforms.
///
/// PORT: the platforms are returned as shared `Arc`s (Java returns the `PlatformInfo`
/// objects themselves).
// Java: com/google/devtools/build/lib/analysis/ToolchainContext.java:23 ToolchainContext
pub trait ToolchainContext: Send + Sync {
    /// Returns the key that identifies this context.
    // Java: com/google/devtools/build/lib/analysis/ToolchainContext.java:25 ToolchainContext.key
    fn key(&self) -> &ToolchainContextKey;

    /// Returns the selected execution platform that these toolchains use.
    // Java: com/google/devtools/build/lib/analysis/ToolchainContext.java:28 ToolchainContext.executionPlatform
    fn execution_platform(&self) -> &Arc<PlatformInfo>;

    /// Returns the target platform that these toolchains generate output for.
    // Java: com/google/devtools/build/lib/analysis/ToolchainContext.java:31 ToolchainContext.targetPlatform
    fn target_platform(&self) -> &Arc<PlatformInfo>;

    /// Returns the toolchain types that were requested.
    // Java: com/google/devtools/build/lib/analysis/ToolchainContext.java:34 ToolchainContext.toolchainTypes
    fn toolchain_types(&self) -> &[ToolchainTypeRequirement];

    /// Returns the labels of the specific toolchains being used.
    // Java: com/google/devtools/build/lib/analysis/ToolchainContext.java:37 ToolchainContext.resolvedToolchainLabels
    fn resolved_toolchain_labels(&self) -> Vec<Label>;
}

impl<T: ToolchainContext + ?Sized> ToolchainContext for Arc<T> {
    fn key(&self) -> &ToolchainContextKey {
        (**self).key()
    }
    fn execution_platform(&self) -> &Arc<PlatformInfo> {
        (**self).execution_platform()
    }
    fn target_platform(&self) -> &Arc<PlatformInfo> {
        (**self).target_platform()
    }
    fn toolchain_types(&self) -> &[ToolchainTypeRequirement] {
        (**self).toolchain_types()
    }
    fn resolved_toolchain_labels(&self) -> Vec<Label> {
        (**self).resolved_toolchain_labels()
    }
}
