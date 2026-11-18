//! Port of `rules/objc/ObjcCommandLineOptions.java`.

use super::dotted_version::{DottedVersionConverter, DottedVersionOption};
use blaze_options::CommaSeparatedOptionListConverter;
use blaze_options::options;

use super::fragment_options::FragmentOptions;

// Java: com/google/devtools/build/lib/rules/objc/ObjcCommandLineOptions.java:28 ObjcCommandLineOptions
options! {
    /// Command-line options for building Objective-C targets.
    pub struct ObjcCommandLineOptions {
        #[option(
            name = "ios_simulator_version",
            default_value = "null",
            converter = DottedVersionConverter,
            documentation_category = TESTING,
            effect_tags = [TEST_RUNNER],
            help = concat!(
                "The version of iOS to run on the simulator when running or testing. This is ignored ",
                "for ios_test rules if a target device is specified in the rule.",
            ),
        )]
        pub ios_simulator_version: Option<DottedVersionOption>,

        #[option(
            name = "ios_simulator_device",
            default_value = "null",
            documentation_category = TESTING,
            effect_tags = [TEST_RUNNER],
            help = concat!(
                "The device to simulate when running an iOS application in the simulator, e.g. ",
                "'iPhone 6'. You can get a list of devices by running 'xcrun simctl list ",
                "devicetypes' on the machine the simulator will be run on.",
            ),
        )]
        pub ios_simulator_device: Option<String>,

        #[option(
            name = "ios_memleaks",
            default_value = "false",
            documentation_category = TESTING,
            effect_tags = [ACTION_COMMAND_LINES],
            help = "Enable checking for memory leaks in ios_test targets.",
        )]
        pub run_memleaks: bool,

        #[option(
            name = "experimental_objc_fastbuild_options",
            default_value = "-O0,-DDEBUG=1",
            converter = CommaSeparatedOptionListConverter,
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [ACTION_COMMAND_LINES],
            help = "Uses these strings as objc fastbuild compiler options.",
        )]
        pub fastbuild_options: Vec<String>,

        #[option(
            name = "ios_signing_cert_name",
            default_value = "null",
            documentation_category = SIGNING,
            effect_tags = [ACTION_COMMAND_LINES],
            help = concat!(
                "Certificate name to use for iOS signing. If not set will fall back to provisioning ",
                "profile. May be the certificate's keychain identity preference or (substring) of ",
                "the certificate's common name, as per codesign's man page (SIGNING IDENTITIES).",
            ),
        )]
        pub ios_signing_cert_name: Option<String>,

        #[option(
            name = "objc_debug_with_GLIBCXX",
            default_value = "false",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [ACTION_COMMAND_LINES],
            help = concat!(
                "If set, and compilation mode is set to 'dbg', define GLIBCXX_DEBUG, ",
                " GLIBCXX_DEBUG_PEDANTIC and GLIBCPP_CONCEPT_CHECKS.",
            ),
        )]
        pub debug_with_glibcxx: bool,

        #[option(
            name = "device_debug_entitlements",
            default_value = "true",
            documentation_category = SIGNING,
            effect_tags = [CHANGES_INPUTS],
            help = concat!(
                "If set, and compilation mode is not 'opt', objc apps will include debug entitlements ",
                "when signing.",
            ),
        )]
        pub device_debug_entitlements: bool,

        #[option(
            name = "incompatible_avoid_hardcoded_objc_compilation_flags",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [AFFECTS_OUTPUTS, BAZEL_INTERNAL_CONFIGURATION, EXECUTION, ACTION_COMMAND_LINES],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "Prevents Bazel from adding compiler options to Objective-C compilation actions. Options",
                " set in the crosstool are still applied.",
            ),
        )]
        pub incompatible_avoid_hardcoded_objc_compilation_flags: bool,

        #[option(
            name = "incompatible_disallow_sdk_frameworks_attributes",
            default_value = "false",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [BUILD_FILE_SEMANTICS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "If true, disallow sdk_frameworks and weak_sdk_frameworks attributes in objc_library and",
                "objc_import.",
            ),
        )]
        pub incompatible_disallow_sdk_frameworks_attributes: bool,

        #[option(
            name = "incompatible_objc_alwayslink_by_default",
            default_value = "false",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [BUILD_FILE_SEMANTICS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "If true, make the default value true for alwayslink attributes in objc_library and",
                " objc_import.",
            ),
        )]
        pub incompatible_objc_alwayslink_by_default: bool,

        /// @deprecated delete when we are sure it's not used anywhere.
        #[option(
            name = "incompatible_disable_native_apple_binary_rule",
            default_value = "false",
            documentation_category = INPUT_STRICTNESS,
            effect_tags = [EAGERNESS_TO_EXIT],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "No-op. Kept here for backwards compatibility.",
            deprecated = true,
        )]
        pub incompatible_disable_native_apple_binary_rule: bool,

        #[option(
            name = "incompatible_strip_executable_safely",
            default_value = "false",
            documentation_category = TOOLCHAIN,
            effect_tags = [ACTION_COMMAND_LINES],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "If true, strip action for executables will use flag -x, which does not break dynamic ",
                "symbol resolution.",
            ),
        )]
        pub incompatible_strip_executable_safely: bool,

        #[option(
            name = "incompatible_builtin_objc_strip_action",
            default_value = "true",
            documentation_category = TOOLCHAIN,
            effect_tags = [ACTION_COMMAND_LINES],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "Whether to emit a strip action as part of objc linking.",
        )]
        pub incompatible_builtin_objc_strip_action: bool,
    }
}

impl FragmentOptions for ObjcCommandLineOptions {}

#[cfg(test)]
mod tests {
    use super::super::test_util::{assert_defaults_round_trip, assert_option_names, parse};
    use super::*;

    #[test]
    fn option_names_match_java() {
        assert_option_names::<ObjcCommandLineOptions>(&[
            "ios_simulator_version",
            "ios_simulator_device",
            "ios_memleaks",
            "experimental_objc_fastbuild_options",
            "ios_signing_cert_name",
            "objc_debug_with_GLIBCXX",
            "device_debug_entitlements",
            "incompatible_avoid_hardcoded_objc_compilation_flags",
            "incompatible_disallow_sdk_frameworks_attributes",
            "incompatible_objc_alwayslink_by_default",
            "incompatible_disable_native_apple_binary_rule",
            "incompatible_strip_executable_safely",
            "incompatible_builtin_objc_strip_action",
        ]);
    }

    #[test]
    fn defaults_round_trip() {
        assert_defaults_round_trip::<ObjcCommandLineOptions>(&[]);
    }

    #[test]
    fn spot_check_parses() {
        let o = parse::<ObjcCommandLineOptions>(&["--ios_simulator_version=17.0"]).unwrap();
        assert_eq!(o.ios_simulator_version.unwrap().to_string(), "17.0");
        assert_eq!(
            parse::<ObjcCommandLineOptions>(&["--ios_simulator_version=x.1"])
                .unwrap_err()
                .get_message(),
            "While parsing option --ios_simulator_version=x.1: Dotted version components must \
             all start with the form \\d+([a-z0-9]*?)?(\\d+)? but got 'x.1'"
        );
    }
}
