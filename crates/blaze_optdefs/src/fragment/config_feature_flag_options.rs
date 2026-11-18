//! Port of `rules/config/ConfigFeatureFlagOptions.java`.

use blaze_options::options;

use super::fragment_options::FragmentOptions;

// Java: com/google/devtools/build/lib/rules/config/ConfigFeatureFlagOptions.java:24 ConfigFeatureFlagOptions
options! {
    /// The options fragment which defines options related to tagged trimming of feature flags.
    pub struct ConfigFeatureFlagOptions {
        /// Whether to perform user-guided trimming of feature flags based on the tagging in the
        /// transitive_configs attribute.
        #[option(
            name = "enforce_transitive_configs_for_config_feature_flag",
            documentation_category = UNDOCUMENTED,
            effect_tags = [LOSES_INCREMENTAL_STATE, AFFECTS_OUTPUTS, BUILD_FILE_SEMANTICS, BAZEL_INTERNAL_CONFIGURATION, LOADING_AND_ANALYSIS],
            default_value = "false",
        )]
        pub enforce_transitive_configs_for_config_feature_flag: bool,

        /// If {@code true}, the configuration contains all non-default feature flag values, and any flags
        /// which are not present are known to have their default value; if {@code false}, the
        /// configuration only contains some feature flag values, and all others have been trimmed and so
        /// nothing is known about their values.
        #[option(
            name = "all feature flag values are present (internal)",
            documentation_category = UNDOCUMENTED,
            effect_tags = [LOSES_INCREMENTAL_STATE, AFFECTS_OUTPUTS, BUILD_FILE_SEMANTICS, BAZEL_INTERNAL_CONFIGURATION, LOADING_AND_ANALYSIS],
            metadata_tags = [INTERNAL],
            default_value = "true",
        )]
        pub all_feature_flag_values_are_present: bool,
    }
}

impl FragmentOptions for ConfigFeatureFlagOptions {}

#[cfg(test)]
mod tests {
    use super::super::test_util::{assert_defaults_round_trip, assert_option_names, parse};
    use super::*;

    #[test]
    fn option_names_match_java() {
        assert_option_names::<ConfigFeatureFlagOptions>(&[
            "enforce_transitive_configs_for_config_feature_flag",
            "all feature flag values are present (internal)",
        ]);
    }

    #[test]
    fn defaults_round_trip() {
        assert_defaults_round_trip::<ConfigFeatureFlagOptions>(&[]);
    }

    #[test]
    fn spot_check_parses() {
        let o = parse::<ConfigFeatureFlagOptions>(&[
            "--enforce_transitive_configs_for_config_feature_flag",
        ])
        .unwrap();
        assert!(o.enforce_transitive_configs_for_config_feature_flag);
        assert!(o.all_feature_flag_values_are_present);
    }
}
