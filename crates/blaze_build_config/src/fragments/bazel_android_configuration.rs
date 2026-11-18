//! Port of `rules/android/BazelAndroidConfiguration.java` (the fragment; its options are
//! `blaze_optdefs::fragment::bazel_android_configuration`).

use std::any::Any;
use std::sync::Arc;

use blaze_optdefs::fragment::bazel_android_configuration::BazelAndroidConfigurationOptions;

use crate::build_options::BuildOptions;
use crate::fragment::{
    Fragment, FragmentClass, FragmentClassInfo, InvalidConfigurationException, required,
};
use crate::fragment_options_class::BAZEL_ANDROID_OPTIONS;

// Java: com/google/devtools/build/lib/rules/android/BazelAndroidConfiguration.java:35 BazelAndroidConfiguration
/// Configuration fragment for Android rules that is specific to Bazel.
#[derive(Debug)]
pub struct BazelAndroidConfiguration {
    merge_android_manifest_permissions: bool,
}

/// `BazelAndroidConfiguration.class`.
pub static BAZEL_ANDROID_CONFIGURATION: FragmentClassInfo = FragmentClassInfo {
    java_name: "com.google.devtools.build.lib.rules.android.BazelAndroidConfiguration",
    starlark_name: Some("bazel_android"),
    required_options: || vec![&BAZEL_ANDROID_OPTIONS],
    requires_starlark_options: false,
    constructor: create,
};

fn create(options: &BuildOptions) -> Result<Arc<dyn Fragment>, InvalidConfigurationException> {
    let fragment: Arc<dyn Fragment> = Arc::new(BazelAndroidConfiguration::new(options));
    Ok(fragment)
}

impl BazelAndroidConfiguration {
    // Java: com/google/devtools/build/lib/rules/android/BazelAndroidConfiguration.java:58 BazelAndroidConfiguration.<init>
    pub fn new(build_options: &BuildOptions) -> BazelAndroidConfiguration {
        let options = required::<BazelAndroidConfigurationOptions>(build_options);
        BazelAndroidConfiguration {
            merge_android_manifest_permissions: options.merge_android_manifest_permissions,
        }
    }

    // Java: com/google/devtools/build/lib/rules/android/BazelAndroidConfiguration.java:67 BazelAndroidConfiguration.getMergeAndroidManifestPermissions
    /// The value of --merge_android_manifest_permissions flag.
    pub fn get_merge_android_manifest_permissions(&self) -> bool {
        self.merge_android_manifest_permissions
    }
}

impl Fragment for BazelAndroidConfiguration {
    fn fragment_class(&self) -> FragmentClass {
        &BAZEL_ANDROID_CONFIGURATION
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
