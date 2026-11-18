//! Transcription of `com/google/devtools/build/lib/runtime/mobileinstall/MobileInstallCommand.java` (the `OptionsBase` subclasses it declares).

// ---- transcribed mechanically from the Java @Option annotations; hand-written code follows the marker below ----
#[allow(unused_imports)]
use super::prelude::*;

// Java: com/google/devtools/build/lib/runtime/mobileinstall/MobileInstallCommand.java:100 MobileInstallCommand.Options
options! {
    /// Command line options for the 'mobile-install' command.
    pub struct MobileInstallCommandOptions {
        #[option(
            name = "split_apks",
            default_value = "false",
            documentation_category = OUTPUT_SELECTION,
            effect_tags = [LOADING_AND_ANALYSIS, AFFECTS_OUTPUTS],
            help = concat!(
                "Whether to use split apks to install and update the ",
                "application on the device. Works only with devices with ",
                "Marshmallow or later",
            ),
        )]
        pub split_apks: bool,

        #[option(
            name = "incremental",
            default_value = "false",
            documentation_category = OUTPUT_SELECTION,
            effect_tags = [LOADING_AND_ANALYSIS],
            help = concat!(
                "Whether to do an incremental install. If true, try to avoid unnecessary additional ",
                "work by reading the state of the device the code is to be installed on and using ",
                "that information to avoid unnecessary work. If false (the default), always do a ",
                "full install.",
            ),
        )]
        pub incremental: bool,

        #[option(
            name = "mode",
            default_value = "skylark",
            converter = ModeConverter,
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [LOADING_AND_ANALYSIS, EXECUTION],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            deprecated = true,
            help = "Deprecated no-effect flag. Only skylark mode is still supported.",
        )]
        pub mode: Mode,

        #[option(
            name = "mobile_install_aspect",
            default_value = "@rules_android//mobile_install:mi.bzl",
            documentation_category = UNDOCUMENTED,
            effect_tags = [LOADING_AND_ANALYSIS, CHANGES_INPUTS],
            help = "The aspect to use for mobile-install.",
        )]
        pub mobile_install_aspect: String,

        #[option(
            name = "mobile_install_supported_rules",
            default_value = "android_binary",
            converter = CommaSeparatedOptionListConverter,
            documentation_category = UNDOCUMENTED,
            effect_tags = [LOADING_AND_ANALYSIS],
            help = "The supported rules for mobile-install.",
        )]
        pub mobile_install_supported_rules: Vec<String>,

        #[option(
            name = "run_in_client",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION],
            help = concat!(
                "If true, the mobile-install deployer command will be sent to the bazel client for ",
                "execution. Useful for configurations where the bazel client is on a different ",
                "machine than the bazel server.",
            ),
        )]
        pub run_in_client: bool,
    }
}

#[cfg(test)]
mod generated_tests {
    use super::*;

    /// Option names in Java declaration order (extracted from the Java source).
    const MOBILE_INSTALL_COMMAND_OPTIONS_OPTION_NAMES: &[&str] = &[
        "split_apks",
        "incremental",
        "mode",
        "mobile_install_aspect",
        "mobile_install_supported_rules",
        "run_in_client",
    ];

    #[test]
    fn mobile_install_command_options_option_names() {
        super::super::test_util::check_option_names(
            MobileInstallCommandOptions::class(),
            MOBILE_INSTALL_COMMAND_OPTIONS_OPTION_NAMES,
        );
    }

    #[test]
    fn mobile_install_command_options_defaults_round_trip() {
        super::super::test_util::check_defaults_round_trip::<MobileInstallCommandOptions>(
            SKIP_ROUND_TRIP,
        );
    }
}

// ---- hand-written below this line (preserved by the generator) ----

// Java: com/google/devtools/build/lib/runtime/mobileinstall/MobileInstallCommand.java:82 MobileInstallCommand.Mode
option_enum! {
    /// An enumeration of all the modes that mobile-install supports.
    pub enum Mode { CLASSIC, CLASSIC_INTERNAL_TEST_DO_NOT_USE, SKYLARK }
}

// Java: com/google/devtools/build/lib/runtime/mobileinstall/MobileInstallCommand.java:91 MobileInstallCommand.ModeConverter
enum_converter!(pub ModeConverter, Mode, "mode");

/// Options whose default cannot be re-parsed in a test (none).
#[cfg(test)]
const SKIP_ROUND_TRIP: &[&str] = &[];

#[cfg(test)]
mod tests {
    use super::super::test_util::parse;
    use super::*;

    #[test]
    fn mode() {
        assert_eq!(
            parse::<MobileInstallCommandOptions>(&["--mode=classic"]).mode,
            Mode::CLASSIC
        );
    }
}
