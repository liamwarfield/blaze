//! Transcription of `com/google/devtools/build/lib/outputfilter/OutputFilteringModule.java` (the `OptionsBase` subclasses it declares).

// ---- transcribed mechanically from the Java @Option annotations; hand-written code follows the marker below ----
#[allow(unused_imports)]
use super::prelude::*;

// Java: com/google/devtools/build/lib/outputfilter/OutputFilteringModule.java:38 OutputFilteringModule.Options
options! {
    /// Options controlling output filtering.
    pub struct OutputFilteringModuleOptions {
        #[option(
            name = "auto_output_filter",
            default_value = "none",
            converter = AutoOutputFilterConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "If --output_filter is not specified, then the value for this option is used ",
                "create a filter automatically. Allowed values are 'none' (filter nothing ",
                "/ show everything), 'all' (filter everything / show nothing), 'packages' ",
                "(include output from rules in packages mentioned on the Blaze command line), ",
                "and 'subpackages' (like 'packages', but also include subpackages). For the ",
                "'packages' and 'subpackages' values //java/foo and //javatests/foo are treated ",
                "as one package)'.",
            ),
        )]
        pub auto_output_filter: AutoOutputFilter,
    }
}

#[cfg(test)]
mod generated_tests {
    use super::*;

    /// Option names in Java declaration order (extracted from the Java source).
    const OUTPUT_FILTERING_MODULE_OPTIONS_OPTION_NAMES: &[&str] = &["auto_output_filter"];

    #[test]
    fn output_filtering_module_options_option_names() {
        super::super::test_util::check_option_names(
            OutputFilteringModuleOptions::class(),
            OUTPUT_FILTERING_MODULE_OPTIONS_OPTION_NAMES,
        );
    }

    #[test]
    fn output_filtering_module_options_defaults_round_trip() {
        super::super::test_util::check_defaults_round_trip::<OutputFilteringModuleOptions>(
            SKIP_ROUND_TRIP,
        );
    }
}

// ---- hand-written below this line (preserved by the generator) ----

/// Options whose default cannot be re-parsed in a test (none).
#[cfg(test)]
const SKIP_ROUND_TRIP: &[&str] = &[];

#[cfg(test)]
mod tests {
    use super::super::test_util::parse;
    use super::*;

    #[test]
    fn auto_output_filter() {
        let o = parse::<OutputFilteringModuleOptions>(&["--auto_output_filter=subpackages"]);
        assert_eq!(o.auto_output_filter, AutoOutputFilter::SUBPACKAGES);
        assert_eq!(
            OutputFilteringModuleOptions::default().auto_output_filter,
            AutoOutputFilter::NONE
        );
    }
}
