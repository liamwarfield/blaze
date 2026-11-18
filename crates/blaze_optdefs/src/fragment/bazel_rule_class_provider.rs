//! The options of `bazel/rules/BazelRuleClassProvider.java` (`BazelRuleClassProvider.StrictActionEnvOptions`). The rule class provider itself is not ported here.

use blaze_options::options;

use super::fragment_options::FragmentOptions;

// Java: com/google/devtools/build/lib/bazel/rules/BazelRuleClassProvider.java:72 BazelRuleClassProvider.StrictActionEnvOptions
options! {
    /// Command-line options.
    pub struct BazelRuleClassProviderStrictActionEnvOptions {
        #[option(
            name = "incompatible_strict_action_env",
            old_name = "experimental_strict_action_env",
            default_value = "true",
            documentation_category = UNCATEGORIZED,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "If true, Bazel uses an environment with a static value for PATH and does not\n",
                "inherit `LD_LIBRARY_PATH`. Use `--action_env=ENV_VARIABLE` if you want to\n",
                "inherit specific environment variables from the client, but note that doing so\n",
                "can prevent cross-user caching if a shared cache is used.\n",
            ),
        )]
        pub use_strict_action_env: bool,
    }
}

impl FragmentOptions for BazelRuleClassProviderStrictActionEnvOptions {}

#[cfg(test)]
mod tests {
    use super::super::test_util::{assert_defaults_round_trip, assert_option_names, parse};
    use super::*;

    #[test]
    fn option_names_match_java() {
        assert_option_names::<BazelRuleClassProviderStrictActionEnvOptions>(&[
            "incompatible_strict_action_env",
        ]);
    }

    #[test]
    fn defaults_round_trip() {
        assert_defaults_round_trip::<BazelRuleClassProviderStrictActionEnvOptions>(&[]);
    }

    #[test]
    fn spot_check_parses() {
        let o = parse::<BazelRuleClassProviderStrictActionEnvOptions>(&[
            "--incompatible_strict_action_env",
        ])
        .unwrap();
        assert!(o.use_strict_action_env);
        let o = parse::<BazelRuleClassProviderStrictActionEnvOptions>(&[
            "--noincompatible_strict_action_env",
        ])
        .unwrap();
        assert!(!o.use_strict_action_env);
    }
}
