//! Port of `analysis/BuildSettingProvider.java`.

use std::any::Any;

use blaze_cmdline::Label;
use blaze_packages::rule_class::BuildSetting;
use blaze_packages::types::{AttrValue, Type};

use crate::provider_map::TransitiveInfoProvider;

/// A native provider to allow select()s to know the type, default value, and label of a build
/// setting (Java `BuildSettingProvider`).
// Java: com/google/devtools/build/lib/analysis/BuildSettingProvider.java:28 BuildSettingProvider
#[derive(Clone, Debug)]
pub struct BuildSettingProvider {
    build_setting: BuildSetting,
    default_value: Option<AttrValue>,
    label: Label,
}

impl BuildSettingProvider {
    /// Java `REQUIRE_BUILD_SETTING_PROVIDER`: the native provider class every build setting
    /// target carries. `target.satisfies(REQUIRE_BUILD_SETTING_PROVIDER)` is
    /// [`BuildSettingProvider::satisfied_by`].
    // Java: com/google/devtools/build/lib/analysis/BuildSettingProvider.java:30 BuildSettingProvider.REQUIRE_BUILD_SETTING_PROVIDER
    pub fn satisfied_by(target: &crate::configured_target::ConfiguredTarget) -> bool {
        target.get_provider::<BuildSettingProvider>().is_some()
    }

    // Java: com/google/devtools/build/lib/analysis/BuildSettingProvider.java:39 BuildSettingProvider.<init>
    pub fn new(
        build_setting: BuildSetting,
        default_value: Option<AttrValue>,
        label: Label,
    ) -> BuildSettingProvider {
        BuildSettingProvider {
            build_setting,
            default_value,
            label,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/BuildSettingProvider.java:45 BuildSettingProvider.getType
    pub fn get_type(&self) -> Type {
        self.build_setting.get_type()
    }

    // Java: com/google/devtools/build/lib/analysis/BuildSettingProvider.java:49 BuildSettingProvider.allowsMultiple
    pub fn allows_multiple(&self) -> bool {
        self.build_setting.allows_multiple()
    }

    // Java: com/google/devtools/build/lib/analysis/BuildSettingProvider.java:53 BuildSettingProvider.getDefaultValue
    pub fn get_default_value(&self) -> Option<&AttrValue> {
        self.default_value.as_ref()
    }

    // Java: com/google/devtools/build/lib/analysis/BuildSettingProvider.java:57 BuildSettingProvider.getLabel
    pub fn get_label(&self) -> &Label {
        &self.label
    }
}

impl TransitiveInfoProvider for BuildSettingProvider {
    fn java_class_name(&self) -> &'static str {
        "BuildSettingProvider"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
