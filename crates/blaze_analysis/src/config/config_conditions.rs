//! Port of `analysis/config/ConfigConditions.java`.

use blaze_cmdline::Label;
use blaze_packages::starlark_defs::platform_common::CONSTRAINT_VALUE_INFO;
use indexmap::IndexMap;

use crate::config::config_matching_provider::ConfigMatchingProvider;
use crate::configured_target_and_data::ConfiguredTargetAndData;
use crate::info::downcast_info;
use crate::platform::constraint_value_info::ConstraintValueInfo;
use crate::platform::platform_info::PlatformInfo;

/// Utility class for temporarily tracking `select()` keys' [`ConfigMatchingProvider`]s and
/// configured targets.
///
/// Its only purpose is to keep the configured targets long enough for `RuleContext.Builder` to do
/// prerequisite validation on them (for example, visibility checks).
// Java: com/google/devtools/build/lib/analysis/config/ConfigConditions.java:37 ConfigConditions
#[derive(Clone, Debug, Default)]
pub struct ConfigConditions {
    as_configured_targets: IndexMap<Label, ConfiguredTargetAndData>,
    as_providers: IndexMap<Label, ConfigMatchingProvider>,
}

/// Exception for when a `select()` has an invalid key (for example, wrong target type).
// Java: com/google/devtools/build/lib/analysis/config/ConfigConditions.java:56 ConfigConditions.InvalidConditionException
#[derive(Clone, Debug, thiserror::Error)]
#[error("InvalidConditionException")]
pub struct InvalidConditionException;

impl ConfigConditions {
    // Java: com/google/devtools/build/lib/analysis/config/ConfigConditions.java:46 ConfigConditions.create
    pub fn create(
        as_configured_targets: IndexMap<Label, ConfiguredTargetAndData>,
        as_providers: IndexMap<Label, ConfigMatchingProvider>,
    ) -> ConfigConditions {
        ConfigConditions {
            as_configured_targets,
            as_providers,
        }
    }

    /// Java `ConfigConditions.EMPTY`.
    // Java: com/google/devtools/build/lib/analysis/config/ConfigConditions.java:52 ConfigConditions.EMPTY
    pub fn empty() -> ConfigConditions {
        ConfigConditions::default()
    }

    /// Java `Objects.equals(configConditions, ConfigConditions.EMPTY)`.
    ///
    /// PORT: record equality compares the two maps; `ConfiguredTargetAndData` has identity
    /// equality, so only two empty maps are equal to `EMPTY`'s.
    pub fn is_empty_instance(&self) -> bool {
        self.as_configured_targets.is_empty() && self.as_providers.is_empty()
    }

    pub fn as_configured_targets(&self) -> &IndexMap<Label, ConfiguredTargetAndData> {
        &self.as_configured_targets
    }

    pub fn as_providers(&self) -> &IndexMap<Label, ConfigMatchingProvider> {
        &self.as_providers
    }

    /// Returns a [`ConfigMatchingProvider`] from the given configured target if appropriate,
    /// else an [`InvalidConditionException`].
    ///
    /// This is the canonical place to extract `ConfigMatchingProvider`s from configured targets.
    /// It's not as simple as `ConfiguredTarget.getProvider`.
    // Java: com/google/devtools/build/lib/analysis/config/ConfigConditions.java:66 ConfigConditions.fromConfiguredTarget
    pub fn from_configured_target(
        select_key: &ConfiguredTargetAndData,
        target_platform: Option<&PlatformInfo>,
    ) -> Result<ConfigMatchingProvider, InvalidConditionException> {
        let selectable = select_key.get_configured_target();
        // The below handles config_setting (which natively provides ConfigMatchingProvider) and
        // constraint_value (which needs a custom-built ConfigMatchingProvider).
        if let Some(matching_provider) = selectable.get_provider::<ConfigMatchingProvider>() {
            return Ok(matching_provider.clone());
        }
        let constraint_value_info = selectable.get(&CONSTRAINT_VALUE_INFO.key());
        let constraint_value_info = constraint_value_info
            .as_ref()
            .and_then(|v| downcast_info::<ConstraintValueInfo>(v));
        if let (Some(constraint_value_info), Some(target_platform)) =
            (constraint_value_info, target_platform)
        {
            // If platformInfo == null, that means the owning target doesn't invoke toolchain
            // resolution, in which case depending on a constraint_value is nonsensical.
            return Ok(constraint_value_info.config_matching_provider(target_platform));
        }

        // Not a valid provider for configuration conditions.
        Err(InvalidConditionException)
    }
}
