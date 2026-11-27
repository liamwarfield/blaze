//! Transcription of `com/google/devtools/build/lib/rules/android/WriteAdbArgsAction.java` (the `OptionsBase` subclasses it declares).

// ---- transcribed mechanically from the Java @Option annotations; hand-written code follows the marker below ----
#[allow(unused_imports)]
use super::prelude::*;

// Java: com/google/devtools/build/lib/rules/android/WriteAdbArgsAction.java:26 WriteAdbArgsAction.Options
options! {
    /// Options of the {@code mobile-install} command pertaining to the way {@code adb} is invoked.
    pub struct WriteAdbArgsActionOptions {
        #[option(
            name = "adb",
            default_value = "",
            documentation_category = TOOLCHAIN,
            effect_tags = [CHANGES_INPUTS],
            help = concat!(
                "adb binary to use for the 'mobile-install' command. If unspecified, the one in ",
                "the Android SDK specified by the --android_sdk_channel command line option (or ",
                " the default SDK if --android_sdk_channel is not specified) is used.",
            ),
        )]
        pub adb: String,

        #[option(
            name = "adb_arg",
            default_value = "null",
            allow_multiple = true,
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [ACTION_COMMAND_LINES],
            help = "Extra arguments to pass to adb. Usually used to designate a device to install to.",
        )]
        pub adb_args: Vec<String>,

        #[option(
            name = "device",
            default_value = "",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [ACTION_COMMAND_LINES],
            help = "The adb device serial number. If not specified, the first device will be used.",
        )]
        pub device: String,

        #[option(
            name = "incremental_install_verbosity",
            default_value = "",
            documentation_category = LOGGING,
            effect_tags = [BAZEL_MONITORING],
            help = "The verbosity for incremental install. Set to 1 for debug logging.",
        )]
        pub incremental_install_verbosity: String,

        #[option(
            name = "start",
            default_value = "NO",
            converter = StartTypeConverter,
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [EXECUTION],
            help = concat!(
                "How the app should be started after installing it. Set to WARM to preserve ",
                "and restore application state on incremental installs.",
            ),
        )]
        pub start: StartType,

        #[option(
            name = "start_app",
            default_value = "null",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [EXECUTION],
            expansion = ["--start=COLD"],
            help = "Whether to start the app after installing it.",
        )]
        pub start_app: Void,

        #[option(
            name = "debug_app",
            default_value = "null",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [EXECUTION],
            expansion = ["--start=DEBUG"],
            help = "Whether to wait for the debugger before starting the app.",
        )]
        pub debug_app: Void,
    }
}

#[cfg(test)]
mod generated_tests {
    use super::*;

    /// Option names in Java declaration order (extracted from the Java source).
    const WRITE_ADB_ARGS_ACTION_OPTIONS_OPTION_NAMES: &[&str] = &[
        "adb",
        "adb_arg",
        "device",
        "incremental_install_verbosity",
        "start",
        "start_app",
        "debug_app",
    ];

    #[test]
    fn write_adb_args_action_options_option_names() {
        super::super::test_util::check_option_names(
            WriteAdbArgsActionOptions::class(),
            WRITE_ADB_ARGS_ACTION_OPTIONS_OPTION_NAMES,
        );
    }

    #[test]
    fn write_adb_args_action_options_defaults_round_trip() {
        super::super::test_util::check_defaults_round_trip::<WriteAdbArgsActionOptions>(
            SKIP_ROUND_TRIP,
        );
    }
}

// ---- hand-written below this line (preserved by the generator) ----

// Java: com/google/devtools/build/lib/rules/android/WriteAdbArgsAction.java:101 WriteAdbArgsAction.StartType
option_enum! {
    /// Specifies how the app should be started/stopped.
    // NO: the app will not be restarted after install. COLD: the app will be restarted from a
    // clean state after install. WARM: the app will save its state before installing, and be
    // restored from that state after installing. DEBUG: the app will wait for debugger to attach
    // before restarting from clean state after install.
    pub enum StartType { NO, COLD, WARM, DEBUG }
}

// Java: com/google/devtools/build/lib/rules/android/WriteAdbArgsAction.java:116 WriteAdbArgsAction.StartTypeConverter
// Java: com/google/devtools/build/lib/rules/android/WriteAdbArgsAction.java:117 WriteAdbArgsAction.StartTypeConverter.<init>
// PORT: skipped, the private constructor of a holder class (never instantiated): WriteAdbArgsAction.<init>
enum_converter!(pub StartTypeConverter, StartType, "start type");

/// Options whose default cannot be re-parsed in a test (none).
#[cfg(test)]
const SKIP_ROUND_TRIP: &[&str] = &[];

#[cfg(test)]
mod tests {
    use super::super::test_util::parse;
    use super::*;

    #[test]
    fn start_expansions() {
        assert_eq!(WriteAdbArgsActionOptions::default().start, StartType::NO);
        assert_eq!(
            parse::<WriteAdbArgsActionOptions>(&["--start_app"]).start,
            StartType::COLD
        );
        assert_eq!(
            parse::<WriteAdbArgsActionOptions>(&["--debug_app"]).start,
            StartType::DEBUG
        );
        let o = parse::<WriteAdbArgsActionOptions>(&["--adb_arg=-s", "--adb_arg=x"]);
        assert_eq!(o.adb_args, vec!["-s", "x"]);
    }
}
