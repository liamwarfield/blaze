//! Port of `analysis/IncompatiblePlatformProvider.java`.

use std::fmt;

use allocative::Allocative;
use blaze_cmdline::Label;
use blaze_packages::starlark_defs::builtin_providers::BuiltinProviderId;
use starlark::any::ProvidesStaticType;
use starlark::starlark_simple_value;
use starlark::values::{Demand, NoSerialize, StarlarkValue, starlark_value};

use crate::configured_target::ConfiguredTarget;
use crate::info::InfoProviderKey;
use crate::platform::constraint_value_info::ConstraintValueInfo;

/// Java `IncompatiblePlatformProvider.PROVIDER` (`BuiltinProvider` named
/// `IncompatiblePlatformProvider`).
// Java: com/google/devtools/build/lib/analysis/IncompatiblePlatformProvider.java:56 IncompatiblePlatformProvider.PROVIDER
pub const INCOMPATIBLE_PLATFORM_PROVIDER: BuiltinProviderId = BuiltinProviderId {
    name: "IncompatiblePlatformProvider",
    value_class: "IncompatiblePlatformProvider",
};

/// Provider instance for the `target_compatible_with` attribute: its presence marks a target
/// incompatible with the target platform.
///
/// `target_platform` is `None` when the incompatibility comes from a target that did not do
/// toolchain resolution.
// Java: com/google/devtools/build/lib/analysis/IncompatiblePlatformProvider.java:47 IncompatiblePlatformProvider
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct IncompatiblePlatformProvider {
    #[allocative(skip)]
    target_platform: Option<Label>,
    #[allocative(skip)]
    targets_responsible_for_incompatibility: Option<Vec<ConfiguredTarget>>,
    #[allocative(skip)]
    constraints_responsible_for_incompatibility: Option<Vec<ConstraintValueInfo>>,
    #[allocative(skip)]
    config_settings_responsible_for_incompatibility: Option<Vec<Label>>,
}

starlark_simple_value!(IncompatiblePlatformProvider);

impl IncompatiblePlatformProvider {
    // Java: com/google/devtools/build/lib/analysis/IncompatiblePlatformProvider.java:46 IncompatiblePlatformProvider.STARLARK_NAME
    pub const STARLARK_NAME: &'static str = "IncompatiblePlatformProvider";

    // Java: com/google/devtools/build/lib/analysis/IncompatiblePlatformProvider.java:66 IncompatiblePlatformProvider.incompatibleDueToTargets
    pub fn incompatible_due_to_targets(
        target_platform: Option<Label>,
        targets_responsible_for_incompatibility: Vec<ConfiguredTarget>,
    ) -> IncompatiblePlatformProvider {
        blaze_util::check_argument!(!targets_responsible_for_incompatibility.is_empty());
        IncompatiblePlatformProvider {
            target_platform,
            targets_responsible_for_incompatibility: Some(targets_responsible_for_incompatibility),
            constraints_responsible_for_incompatibility: None,
            config_settings_responsible_for_incompatibility: None,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/IncompatiblePlatformProvider.java:75 IncompatiblePlatformProvider.incompatibleDueToConstraints
    pub fn incompatible_due_to_constraints(
        target_platform: Option<Label>,
        constraints: Vec<ConstraintValueInfo>,
    ) -> IncompatiblePlatformProvider {
        Self::incompatible_due_to_constraints_and_config_settings(
            target_platform,
            constraints,
            Vec::new(),
        )
    }

    // Java: com/google/devtools/build/lib/analysis/IncompatiblePlatformProvider.java:89 IncompatiblePlatformProvider.incompatibleDueToConstraintsAndConfigSettings
    pub fn incompatible_due_to_constraints_and_config_settings(
        target_platform: Option<Label>,
        constraints: Vec<ConstraintValueInfo>,
        config_settings: Vec<Label>,
    ) -> IncompatiblePlatformProvider {
        blaze_util::check_argument!(!constraints.is_empty() || !config_settings.is_empty());

        // Deduplicate and sort the list of incompatible constraints. Doing it here means that
        // everyone inspecting this provider doesn't have to deal with it.
        let mut constraints = constraints;
        constraints.sort_by(|a, b| a.label().cmp(b.label()));
        constraints.dedup();

        let mut config_settings = config_settings;
        config_settings.sort();
        config_settings.dedup();

        IncompatiblePlatformProvider {
            target_platform,
            targets_responsible_for_incompatibility: None,
            constraints_responsible_for_incompatibility: if constraints.is_empty() {
                None
            } else {
                Some(constraints)
            },
            config_settings_responsible_for_incompatibility: if config_settings.is_empty() {
                None
            } else {
                Some(config_settings)
            },
        }
    }

    pub fn target_platform(&self) -> Option<&Label> {
        self.target_platform.as_ref()
    }

    pub fn targets_responsible_for_incompatibility(&self) -> Option<&[ConfiguredTarget]> {
        self.targets_responsible_for_incompatibility.as_deref()
    }

    pub fn constraints_responsible_for_incompatibility(&self) -> Option<&[ConstraintValueInfo]> {
        self.constraints_responsible_for_incompatibility.as_deref()
    }

    pub fn config_settings_responsible_for_incompatibility(&self) -> Option<&[Label]> {
        self.config_settings_responsible_for_incompatibility
            .as_deref()
    }
}

impl fmt::Display for IncompatiblePlatformProvider {
    // PORT: Java's record `toString` is not reachable from Starlark output (`repr` of a
    // NativeInfo without fields); print the provider name.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<instance of provider {}>", Self::STARLARK_NAME)
    }
}

#[starlark_value(type = "IncompatiblePlatformProvider")]
impl<'v> StarlarkValue<'v> for IncompatiblePlatformProvider {
    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(InfoProviderKey(INCOMPATIBLE_PLATFORM_PROVIDER.key()));
    }
}
