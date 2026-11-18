//! The options of `rules/android/BazelAndroidConfiguration.java` (`BazelAndroidConfiguration.Options`). The fragment itself is not ported here.

use blaze_options::options;

use super::fragment_options::FragmentOptions;

// Java: com/google/devtools/build/lib/rules/android/BazelAndroidConfiguration.java:43 BazelAndroidConfiguration.Options
options! {
    /// Android configuration options that are specific to Bazel.
    pub struct BazelAndroidConfigurationOptions {
        #[option(
            name = "merge_android_manifest_permissions",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "If enabled, the manifest merger will merge uses-permission and ",
                "uses-permission-sdk-23 attributes.",
            ),
        )]
        pub merge_android_manifest_permissions: bool,
    }
}

impl FragmentOptions for BazelAndroidConfigurationOptions {}

#[cfg(test)]
mod tests {
    use super::super::test_util::{assert_defaults_round_trip, assert_option_names, parse};
    use super::*;

    #[test]
    fn option_names_match_java() {
        assert_option_names::<BazelAndroidConfigurationOptions>(&[
            "merge_android_manifest_permissions",
        ]);
    }

    #[test]
    fn defaults_round_trip() {
        assert_defaults_round_trip::<BazelAndroidConfigurationOptions>(&[]);
    }

    #[test]
    fn spot_check_parses() {
        let o =
            parse::<BazelAndroidConfigurationOptions>(&["--merge_android_manifest_permissions"])
                .unwrap();
        assert!(o.merge_android_manifest_permissions);
    }
}
