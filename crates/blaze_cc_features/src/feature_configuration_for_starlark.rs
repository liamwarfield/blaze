//! Port of `com.google.devtools.build.lib.rules.cpp.FeatureConfigurationForStarlark`.

use std::sync::Arc;

use crate::cc_toolchain_features::FeatureConfiguration;

/// Wrapper for [`FeatureConfiguration`] handed to Starlark by
/// `cc_common.configure_features(ctx, cc_toolchain)`.
///
/// The Starlark wrapper crate exposes `is_enabled` / `is_requested` (after the private-API
/// allowlist check that `CcModule.checkPrivateStarlarkificationAllowlist` does) and uses
/// [`FeatureConfigurationForStarlark::str`] / [`FeatureConfigurationForStarlark::debug_print`] for
/// `str()` and `print()`.
// Java: com/google/devtools/build/lib/rules/cpp/FeatureConfigurationForStarlark.java:38 FeatureConfigurationForStarlark
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeatureConfigurationForStarlark {
    feature_configuration: Arc<FeatureConfiguration>,
}

impl FeatureConfigurationForStarlark {
    // Java: com/google/devtools/build/lib/rules/cpp/FeatureConfigurationForStarlark.java:42 FeatureConfigurationForStarlark.from
    pub fn from(
        feature_configuration: Arc<FeatureConfiguration>,
    ) -> FeatureConfigurationForStarlark {
        FeatureConfigurationForStarlark {
            feature_configuration,
        }
    }

    // Java: com/google/devtools/build/lib/rules/cpp/FeatureConfigurationForStarlark.java:50 FeatureConfigurationForStarlark.getFeatureConfiguration
    pub fn get_feature_configuration(&self) -> &Arc<FeatureConfiguration> {
        &self.feature_configuration
    }

    // Java: com/google/devtools/build/lib/rules/cpp/FeatureConfigurationForStarlark.java:55 FeatureConfigurationForStarlark.str
    pub fn str(&self) -> String {
        let fc = &self.feature_configuration;
        let enabled: Vec<&str> = fc
            .get_enabled_feature_names()
            .iter()
            .map(|s| s.as_str())
            .collect();
        let requested: Vec<&str> = fc
            .get_requested_features()
            .iter()
            .map(|s| s.as_str())
            .collect();
        format!(
            "<FeatureConfiguration(ENABLED:{};REQUESTED:{})>",
            enabled.join(", "),
            requested.join(", ")
        )
    }

    // Java: com/google/devtools/build/lib/rules/cpp/FeatureConfigurationForStarlark.java:65 FeatureConfigurationForStarlark.debugPrint
    pub fn debug_print(&self) -> String {
        let enabled: Vec<&str> = self
            .feature_configuration
            .get_enabled_feature_names()
            .iter()
            .map(|s| s.as_str())
            .collect();
        format!("<FeatureConfiguration({})>", enabled.join(", "))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/FeatureConfigurationForStarlark.java:76 FeatureConfigurationForStarlark.isRequested
    pub fn is_requested(&self, feature: &str) -> bool {
        self.feature_configuration
            .get_requested_features()
            .contains(feature)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/FeatureConfigurationForStarlark.java:87 FeatureConfigurationForStarlark.isEnabled
    pub fn is_enabled(&self, feature: &str) -> bool {
        self.feature_configuration.is_enabled(feature)
    }
}
