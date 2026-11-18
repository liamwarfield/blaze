//! Port of `bazel/coverage/BazelCoverageReportModule.java` (the `OptionsBase` subclass and what it uses).

use blaze_options::{enum_converter, option_enum, options};

// Java: com/google/devtools/build/lib/bazel/coverage/BazelCoverageReportModule.java:60 BazelCoverageReportModule.ReportType
option_enum! {
    /// Possible values for the --combined_report option.
    pub enum ReportType { NONE, LCOV }
}

// Java: com/google/devtools/build/lib/bazel/coverage/BazelCoverageReportModule.java:66 BazelCoverageReportModule.ReportTypeConverter
enum_converter!(
    pub ReportTypeConverter,
    ReportType,
    "combined coverage report type"
);

// Java: com/google/devtools/build/lib/bazel/coverage/BazelCoverageReportModule.java:45 BazelCoverageReportModule.Options
options! {
    /// Options that affect coverage report generation.
    pub struct BazelCoverageReportModuleOptions {
        #[option(
            name = "combined_report",
            converter = ReportTypeConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            default_value = "lcov",
            help = concat!(
                "Specifies desired cumulative coverage report type. At this point only LCOV ",
                "is supported.",
            ),
        )]
        pub combined_report: ReportType,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::misc::test_support::{check_options_class, parse};

    #[test]
    fn options_class_matches_java() {
        check_options_class::<BazelCoverageReportModuleOptions>(&["combined_report"]);
    }

    #[test]
    fn parses_flag() {
        let o = parse::<BazelCoverageReportModuleOptions>(&["--combined_report=none"]).unwrap();
        assert_eq!(o.combined_report, ReportType::NONE);
        assert_eq!(
            BazelCoverageReportModuleOptions::default().combined_report,
            ReportType::LCOV
        );
    }
}
