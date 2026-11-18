//! Port of `metrics/MetricsModule.java` (the `OptionsBase` subclass and what it uses).

use blaze_options::options;

// Java: com/google/devtools/build/lib/metrics/MetricsModule.java:32 MetricsModule.Options
options! {
    /// Metrics options.
    pub struct MetricsModuleOptions {
        #[option(
            name = "experimental_record_metrics_for_all_mnemonics",
            default_value = "false",
            documentation_category = LOGGING,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Controls the output of BEP ActionSummary and BuildGraphMetrics, limiting the number of",
                " mnemonics in ActionData and number of entries reported in",
                " BuildGraphMetrics.AspectCount/RuleClassCount. By default the number of types is",
                " limited to the top 20, by number of executed actions for ActionData, and",
                " instances for RuleClass and Asepcts. Setting this option will write statistics",
                " for all mnemonics, rule classes and aspects.",
            ),
        )]
        pub record_metrics_for_all_mnemonics: bool,

        #[option(
            name = "experimental_record_skyframe_metrics",
            default_value = "false",
            documentation_category = LOGGING,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Controls the output of BEP BuildGraphMetrics, including expensive",
                "to compute skyframe metrics about Skykeys, RuleClasses and Aspects.",
                "With this flag set to false BuildGraphMetrics.rule_count and aspect",
                "fields will not be populated in the BEP.",
            ),
        )]
        pub record_skyframe_metrics: bool,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::misc::test_support::{check_options_class, parse};

    #[test]
    fn options_class_matches_java() {
        check_options_class::<MetricsModuleOptions>(&[
            "experimental_record_metrics_for_all_mnemonics",
            "experimental_record_skyframe_metrics",
        ]);
    }

    #[test]
    fn parses_flags() {
        let o = parse::<MetricsModuleOptions>(&[
            "--experimental_record_metrics_for_all_mnemonics",
            "--noexperimental_record_skyframe_metrics",
        ])
        .unwrap();
        assert!(o.record_metrics_for_all_mnemonics);
        assert!(!o.record_skyframe_metrics);
    }
}
