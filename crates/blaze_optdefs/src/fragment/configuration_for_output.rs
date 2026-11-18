//! `ConfigurationForOutput.UserDefinedFragment` (`analysis/config/output/ConfigurationForOutput.java`),
//! the only other `FragmentOptions` subclass in Bazel 9.3.2. It declares no options.

use blaze_options::options;

use super::fragment_options::FragmentOptions;

// Java: com/google/devtools/build/lib/analysis/config/output/ConfigurationForOutput.java:259 ConfigurationForOutput.UserDefinedFragment
options! {
    /// Starlark options don't have configuration fragments. This is just to keep their output
    /// consistent with native options, i.e. to include "user-defined" section in the output list.
    pub struct ConfigurationForOutputUserDefinedFragment {
        // Intentionally empty: we read the actual options directly from BuildOptions.
    }
}

impl ConfigurationForOutputUserDefinedFragment {
    // Java: com/google/devtools/build/lib/analysis/config/output/ConfigurationForOutput.java:260 ConfigurationForOutput.UserDefinedFragment.DESCRIPTIVE_NAME
    pub const DESCRIPTIVE_NAME: &'static str = "user-defined";
}

impl FragmentOptions for ConfigurationForOutputUserDefinedFragment {}

#[cfg(test)]
mod tests {
    use super::super::test_util::{assert_defaults_round_trip, assert_option_names};
    use super::*;

    #[test]
    fn option_names_match_java() {
        assert_option_names::<ConfigurationForOutputUserDefinedFragment>(&[]);
    }

    #[test]
    fn defaults_round_trip() {
        assert_defaults_round_trip::<ConfigurationForOutputUserDefinedFragment>(&[]);
    }
}
