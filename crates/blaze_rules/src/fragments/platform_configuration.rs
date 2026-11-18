//! The Starlark value of the `platform` fragment (`analysis/PlatformConfiguration.java`,
//! `starlarkbuildapi/platform/PlatformConfigurationApi.java`).

use blaze_build_config::fragments::platform_configuration::{
    PLATFORM_CONFIGURATION, PlatformConfiguration,
};
use blaze_starlark::label::LabelValue;
use starlark::environment::MethodsBuilder;
use starlark::starlark_module;

use super::fragment_value;

fragment_value!(
    /// `ctx.fragments.platform`: the platform configuration.
    // Java: com/google/devtools/build/lib/analysis/PlatformConfiguration.java:33 PlatformConfiguration
    PlatformConfigurationValue,
    PlatformConfiguration,
    "platform",
    "com.google.devtools.build.lib.analysis.PlatformConfiguration",
    platform_configuration_methods
);

pub(super) fn register() {
    super::register(&PLATFORM_CONFIGURATION, PlatformConfigurationValue::alloc);
}

#[starlark_module]
fn platform_configuration_methods(builder: &mut MethodsBuilder) {
    /// The current host platform
    // Java: com/google/devtools/build/lib/analysis/PlatformConfiguration.java:66 PlatformConfiguration.getHostPlatform
    #[starlark(attribute)]
    fn host_platform(this: &PlatformConfigurationValue) -> starlark::Result<LabelValue> {
        Ok(LabelValue(this.fragment().get_host_platform().clone()))
    }

    /// The current target platform
    // Java: com/google/devtools/build/lib/analysis/PlatformConfiguration.java:84 PlatformConfiguration.getTargetPlatform
    #[starlark(attribute)]
    fn platform(this: &PlatformConfigurationValue) -> starlark::Result<LabelValue> {
        Ok(LabelValue(this.fragment().get_target_platform().clone()))
    }

    /// The current target platforms
    // Java: com/google/devtools/build/lib/analysis/PlatformConfiguration.java:89 PlatformConfiguration.getTargetPlatforms
    #[starlark(attribute)]
    fn platforms(this: &PlatformConfigurationValue) -> starlark::Result<Vec<LabelValue>> {
        Ok(this
            .fragment()
            .get_target_platforms()
            .into_iter()
            .map(LabelValue)
            .collect())
    }
}
