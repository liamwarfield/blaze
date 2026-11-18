//! Port of `rules/objc/J2ObjcCommandLineOptions.java`.

use super::converters::{LabelConverter, OptionLabel};
use blaze_options::CommaSeparatedOptionListConverter;
use blaze_options::options;

use super::fragment_options::FragmentOptions;

// Java: com/google/devtools/build/lib/rules/objc/J2ObjcCommandLineOptions.java:30 J2ObjcCommandLineOptions
options! {
    /// Command-line options for J2ObjC translation of Java source code to ObjC. These command line
    /// options are used by Java rules that can be transpiled (specifically, J2ObjCAspects thereof).
    pub struct J2ObjcCommandLineOptions {
        #[option(
            name = "j2objc_translation_flags",
            converter = CommaSeparatedOptionListConverter,
            allow_multiple = true,
            default_value = "null",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = "Additional options to pass to the J2ObjC tool.",
        )]
        pub translation_flags: Vec<String>,

        #[option(
            name = "j2objc_dead_code_report",
            default_value = "null",
            converter = LabelConverter,
            documentation_category = UNDOCUMENTED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Allows J2ObjC to strip dead code reported by ProGuard. Takes a label that can ",
                "generate a dead code report as argument.",
            ),
        )]
        pub dead_code_report: Option<OptionLabel>,
    }
}

impl FragmentOptions for J2ObjcCommandLineOptions {}

#[cfg(test)]
mod tests {
    use super::super::test_util::{assert_defaults_round_trip, assert_option_names, parse};
    use super::*;

    #[test]
    fn option_names_match_java() {
        assert_option_names::<J2ObjcCommandLineOptions>(&[
            "j2objc_translation_flags",
            "j2objc_dead_code_report",
        ]);
    }

    #[test]
    fn defaults_round_trip() {
        assert_defaults_round_trip::<J2ObjcCommandLineOptions>(&[]);
    }

    #[test]
    fn spot_check_parses() {
        let o = parse::<J2ObjcCommandLineOptions>(&[
            "--j2objc_translation_flags=-a,-b",
            "--j2objc_translation_flags=-c",
            "--j2objc_dead_code_report=//r:report",
        ])
        .unwrap();
        assert_eq!(o.translation_flags, vec!["-a", "-b", "-c"]);
        assert_eq!(o.dead_code_report.unwrap().to_string(), "//r:report");
    }
}
