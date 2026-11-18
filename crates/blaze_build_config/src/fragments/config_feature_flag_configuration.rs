//! Port of `rules/config/ConfigFeatureFlagConfiguration.java`.

use std::any::Any;
use std::sync::Arc;

use crate::build_options::BuildOptions;
use crate::fragment::{Fragment, FragmentClass, FragmentClassInfo, InvalidConfigurationException};
use crate::fragment_options_class::CONFIG_FEATURE_FLAG_OPTIONS;

// Java: com/google/devtools/build/lib/rules/config/ConfigFeatureFlagConfiguration.java:27 ConfigFeatureFlagConfiguration
/// Configuration fragment for Android's config_feature_flag, flags which can be defined in BUILD
/// files. This exists only so that ConfigFeatureFlagOptions.class is retained.
#[derive(Debug)]
pub struct ConfigFeatureFlagConfiguration;

/// `ConfigFeatureFlagConfiguration.class`.
pub static CONFIG_FEATURE_FLAG_CONFIGURATION: FragmentClassInfo = FragmentClassInfo {
    java_name: "com.google.devtools.build.lib.rules.config.ConfigFeatureFlagConfiguration",
    starlark_name: None,
    required_options: || vec![&CONFIG_FEATURE_FLAG_OPTIONS],
    requires_starlark_options: true,
    constructor: create,
};

fn create(options: &BuildOptions) -> Result<Arc<dyn Fragment>, InvalidConfigurationException> {
    let fragment: Arc<dyn Fragment> = Arc::new(ConfigFeatureFlagConfiguration::new(options));
    Ok(fragment)
}

impl ConfigFeatureFlagConfiguration {
    // Java: com/google/devtools/build/lib/rules/config/ConfigFeatureFlagConfiguration.java:29 ConfigFeatureFlagConfiguration.<init>
    /// Creates a new configuration fragment from the given `ConfigFeatureFlagOptions`.
    pub fn new(_build_options: &BuildOptions) -> ConfigFeatureFlagConfiguration {
        ConfigFeatureFlagConfiguration
    }
}

impl Fragment for ConfigFeatureFlagConfiguration {
    fn fragment_class(&self) -> FragmentClass {
        &CONFIG_FEATURE_FLAG_CONFIGURATION
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
