//! Port of `packages/metrics/PackageMetricsModule.java` (the `OptionsBase` subclass and what it uses).

use blaze_options::options;

// Java: com/google/devtools/build/lib/packages/metrics/PackageMetricsModule.java:33 PackageMetricsModule.Options
options! {
    /// Options for `PackageMetricsModule`.
    pub struct PackageMetricsModuleOptions {
        #[option(
            name = "log_top_n_packages",
            default_value = "10",
            documentation_category = UNDOCUMENTED,
            effect_tags = [BAZEL_MONITORING],
            help = "Configures number of packages included in top-package INFO logging, <= 0 disables.",
        )]
        pub number_of_packages_to_track: i32,

        #[option(
            name = "record_metrics_for_all_packages",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [BAZEL_MONITORING],
            help = concat!(
                "Configures PackageMetrics to record all metrics for all packages. Disables Top-n INFO",
                " logging.",
            ),
        )]
        pub enable_all_metrics: bool,

        #[option(
            name = "experimental_publish_package_metrics_in_bep",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [BAZEL_MONITORING],
            help = "Whether to publish package metrics in the BEP.",
        )]
        pub publish_package_metrics_in_bep: bool,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::misc::test_support::{check_options_class, parse};

    #[test]
    fn options_class_matches_java() {
        check_options_class::<PackageMetricsModuleOptions>(&[
            "log_top_n_packages",
            "record_metrics_for_all_packages",
            "experimental_publish_package_metrics_in_bep",
        ]);
    }

    #[test]
    fn parses_flags() {
        let o = parse::<PackageMetricsModuleOptions>(&["--log_top_n_packages=3"]).unwrap();
        assert_eq!(o.number_of_packages_to_track, 3);
        assert_eq!(
            parse::<PackageMetricsModuleOptions>(&["--log_top_n_packages=x"])
                .unwrap_err()
                .get_message(),
            "While parsing option --log_top_n_packages=x: 'x' is not an int"
        );
    }
}
