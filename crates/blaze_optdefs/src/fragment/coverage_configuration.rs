//! The options of `analysis/test/CoverageConfiguration.java` (`CoverageConfiguration.CoverageOptions`). The fragment itself is not ported here.

use super::converters::LabelConverter;
use blaze_cmdline::Label;
use blaze_options::Foreign;
use blaze_options::options;

use super::fragment_options::FragmentOptions;

// Java: com/google/devtools/build/lib/analysis/test/CoverageConfiguration.java:38 CoverageConfiguration.CoverageOptions
options! {
    /// Command-line options.
    pub struct CoverageConfigurationCoverageOptions {
        #[option(
            name = "coverage_output_generator",
            converter = LabelConverter,
            default_value = "@bazel_tools//tools/test:lcov_merger",
            documentation_category = TOOLCHAIN,
            effect_tags = [CHANGES_INPUTS, AFFECTS_OUTPUTS, LOADING_AND_ANALYSIS],
            help = concat!(
                "Location of the binary that is used to postprocess raw coverage reports. This must\n",
                "be a binary target. Defaults to `@bazel_tools//tools/test:lcov_merger`.\n",
            ),
        )]
        pub coverage_output_generator: Foreign<Label>,

        #[option(
            name = "coverage_report_generator",
            converter = LabelConverter,
            default_value = "@bazel_tools//tools/test:coverage_report_generator",
            documentation_category = TOOLCHAIN,
            effect_tags = [CHANGES_INPUTS, AFFECTS_OUTPUTS, LOADING_AND_ANALYSIS],
            help = concat!(
                "Location of the binary that is used to generate coverage reports. This must\n",
                "be a binary target. Defaults to `@bazel_tools//tools/test:coverage_report_generator`.\n",
            ),
        )]
        pub coverage_report_generator: Foreign<Label>,
    }
}

impl FragmentOptions for CoverageConfigurationCoverageOptions {}

#[cfg(test)]
mod tests {
    use super::super::test_util::{assert_defaults_round_trip, assert_option_names, parse};
    use super::*;

    #[test]
    fn option_names_match_java() {
        assert_option_names::<CoverageConfigurationCoverageOptions>(&[
            "coverage_output_generator",
            "coverage_report_generator",
        ]);
    }

    #[test]
    fn defaults_round_trip() {
        assert_defaults_round_trip::<CoverageConfigurationCoverageOptions>(&[]);
    }

    #[test]
    fn spot_check_parses() {
        let o = parse::<CoverageConfigurationCoverageOptions>(&[
            "--coverage_report_generator=//my:gen",
        ])
        .unwrap();
        assert_eq!(o.coverage_report_generator.to_string(), "//my:gen");
        assert!(
            parse::<CoverageConfigurationCoverageOptions>(&["--coverage_output_generator="])
                .is_err()
        );
    }
}
