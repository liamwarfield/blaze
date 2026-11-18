//! Port of `rules/apple/AppleCommandLineOptions.java`.

use blaze_options::{
    CommaSeparatedOptionListConverter, ConversionContext, Converter, Value, converter::non_null,
    options,
};

use super::apple_configuration::AppleConfigurationConfigurationDistinguisher;
use super::apple_platform::apple_platform_platform_type as platform_type;
use super::converters::{ConvResult, LabelConverter, LabelListConverter, OptionLabel};
use super::dotted_version::{DottedVersion, DottedVersionConverter, DottedVersionOption};
use super::fragment_options::FragmentOptions;

// Java: com/google/devtools/build/lib/rules/apple/AppleCommandLineOptions.java:170 AppleCommandLineOptions.DEFAULT_IOS_SDK_VERSION
pub const DEFAULT_IOS_SDK_VERSION: &str = "8.4";
// Java: com/google/devtools/build/lib/rules/apple/AppleCommandLineOptions.java:171 AppleCommandLineOptions.DEFAULT_WATCHOS_SDK_VERSION
pub const DEFAULT_WATCHOS_SDK_VERSION: &str = "2.0";
// Java: com/google/devtools/build/lib/rules/apple/AppleCommandLineOptions.java:172 AppleCommandLineOptions.DEFAULT_MACOS_SDK_VERSION
pub const DEFAULT_MACOS_SDK_VERSION: &str = "10.11";
// Java: com/google/devtools/build/lib/rules/apple/AppleCommandLineOptions.java:173 AppleCommandLineOptions.DEFAULT_TVOS_SDK_VERSION
pub const DEFAULT_TVOS_SDK_VERSION: &str = "9.0";
// Java: com/google/devtools/build/lib/rules/apple/AppleCommandLineOptions.java:174 AppleCommandLineOptions.DEFAULT_IOS_CPU
pub const DEFAULT_IOS_CPU: &str = "x86_64";
// Java: com/google/devtools/build/lib/rules/apple/AppleCommandLineOptions.java:177 AppleCommandLineOptions.DEFAULT_VISIONOS_CPU
/// The default visionOS CPU value.
pub const DEFAULT_VISIONOS_CPU: &str = "sim_arm64";
// Java: com/google/devtools/build/lib/rules/apple/AppleCommandLineOptions.java:180 AppleCommandLineOptions.DEFAULT_WATCHOS_CPU
/// The default watchos CPU value.
pub const DEFAULT_WATCHOS_CPU: &str = if cfg!(target_arch = "aarch64") {
    "arm64"
} else {
    "x86_64"
};
// Java: com/google/devtools/build/lib/rules/apple/AppleCommandLineOptions.java:184 AppleCommandLineOptions.DEFAULT_TVOS_CPU
/// The default tvOS CPU value.
pub const DEFAULT_TVOS_CPU: &str = if cfg!(target_arch = "aarch64") {
    "sim_arm64"
} else {
    "x86_64"
};
// Java: com/google/devtools/build/lib/rules/apple/AppleCommandLineOptions.java:188 AppleCommandLineOptions.DEFAULT_MACOS_CPU
/// The default macOS CPU value.
pub const DEFAULT_MACOS_CPU: &str = if cfg!(target_arch = "aarch64") {
    "arm64"
} else {
    "x86_64"
};
// Java: com/google/devtools/build/lib/rules/apple/AppleCommandLineOptions.java:192 AppleCommandLineOptions.DEFAULT_CATALYST_CPU
/// The default Catalyst CPU value.
pub const DEFAULT_CATALYST_CPU: &str = "x86_64";
// Java: com/google/devtools/build/lib/rules/apple/AppleCommandLineOptions.java:326 AppleCommandLineOptions.DEFAULT_XCODE_VERSION_CONFIG_LABEL
/// The default label of the build-wide `xcode_config` configuration rule. This can be changed
/// from the default using the `xcode_version_config` build flag.
pub const DEFAULT_XCODE_VERSION_CONFIG_LABEL: &str = "//tools/objc:host_xcodes";

// Java: com/google/devtools/build/lib/rules/apple/AppleCommandLineOptions.java:36 AppleCommandLineOptions
options! {
    /// Command-line options for building for Apple platforms.
    pub struct AppleCommandLineOptions {
        #[option(
            name = "experimental_objc_provider_from_linked",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "No-op. Kept here for backwards compatibility. This field will be removed in a ",
                "future release.",
            ),
        )]
        pub objc_provider_from_linked: bool,

        #[option(
            name = "xcode_version",
            default_value = "null",
            documentation_category = TOOLCHAIN,
            effect_tags = [LOSES_INCREMENTAL_STATE],
            help = concat!(
                "If specified, uses Xcode of the given version for relevant build actions. ",
                "If unspecified, uses the executor default version of Xcode.",
            ),
        )]
        pub xcode_version: Option<String>,

        #[option(
            name = "ios_sdk_version",
            default_value = "null",
            converter = DottedVersionConverter,
            documentation_category = TOOLCHAIN,
            effect_tags = [LOSES_INCREMENTAL_STATE],
            help = concat!(
                "Specifies the version of the iOS SDK to use to build iOS applications. ",
                "If unspecified, uses the default iOS SDK version from 'xcode_version'.",
            ),
        )]
        pub ios_sdk_version: Option<DottedVersionOption>,

        #[option(
            name = "watchos_sdk_version",
            default_value = "null",
            converter = DottedVersionConverter,
            documentation_category = TOOLCHAIN,
            effect_tags = [LOSES_INCREMENTAL_STATE],
            help = concat!(
                "Specifies the version of the watchOS SDK to use to build watchOS applications. ",
                "If unspecified, uses the default watchOS SDK version from 'xcode_version'.",
            ),
        )]
        pub watch_os_sdk_version: Option<DottedVersionOption>,

        #[option(
            name = "tvos_sdk_version",
            default_value = "null",
            converter = DottedVersionConverter,
            documentation_category = TOOLCHAIN,
            effect_tags = [LOSES_INCREMENTAL_STATE],
            help = concat!(
                "Specifies the version of the tvOS SDK to use to build tvOS applications. ",
                "If unspecified, uses the default tvOS SDK version from 'xcode_version'.",
            ),
        )]
        pub tv_os_sdk_version: Option<DottedVersionOption>,

        #[option(
            name = "macos_sdk_version",
            default_value = "null",
            converter = DottedVersionConverter,
            documentation_category = TOOLCHAIN,
            effect_tags = [LOSES_INCREMENTAL_STATE],
            help = concat!(
                "Specifies the version of the macOS SDK to use to build macOS applications. ",
                "If unspecified, uses the default macOS SDK version from 'xcode_version'.",
            ),
        )]
        pub mac_os_sdk_version: Option<DottedVersionOption>,

        #[option(
            name = "ios_minimum_os",
            default_value = "null",
            converter = DottedVersionConverter,
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [LOSES_INCREMENTAL_STATE],
            help = concat!(
                "Minimum compatible iOS version for target simulators and devices. ",
                "If unspecified, uses 'ios_sdk_version'.",
            ),
        )]
        pub ios_minimum_os: Option<DottedVersionOption>,

        #[option(
            name = "watchos_minimum_os",
            default_value = "null",
            converter = DottedVersionConverter,
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [LOSES_INCREMENTAL_STATE],
            help = concat!(
                "Minimum compatible watchOS version for target simulators and devices. ",
                "If unspecified, uses 'watchos_sdk_version'.",
            ),
        )]
        pub watchos_minimum_os: Option<DottedVersionOption>,

        #[option(
            name = "tvos_minimum_os",
            default_value = "null",
            converter = DottedVersionConverter,
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [LOSES_INCREMENTAL_STATE],
            help = concat!(
                "Minimum compatible tvOS version for target simulators and devices. ",
                "If unspecified, uses 'tvos_sdk_version'.",
            ),
        )]
        pub tvos_minimum_os: Option<DottedVersionOption>,

        #[option(
            name = "macos_minimum_os",
            default_value = "null",
            converter = DottedVersionConverter,
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [LOSES_INCREMENTAL_STATE],
            help = concat!(
                "Minimum compatible macOS version for targets. ",
                "If unspecified, uses 'macos_sdk_version'.",
            ),
        )]
        pub macos_minimum_os: Option<DottedVersionOption>,

        #[option(
            name = "host_macos_minimum_os",
            default_value = "null",
            converter = DottedVersionConverter,
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [LOSES_INCREMENTAL_STATE],
            help = concat!(
                "Minimum compatible macOS version for host targets. ",
                "If unspecified, uses 'macos_sdk_version'.",
            ),
        )]
        pub host_macos_minimum_os: Option<DottedVersionOption>,

        #[option(
            name = "experimental_prefer_mutual_xcode",
            default_value = "true",
            documentation_category = TOOLCHAIN,
            effect_tags = [LOSES_INCREMENTAL_STATE],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "If true, use the most recent Xcode that is available both locally and remotely. If",
                " false, or if there are no mutual available versions, use the local Xcode version",
                " selected via xcode-select.",
            ),
        )]
        pub prefer_mutual_xcode: bool,

        #[option(
            name = "apple_platform_type",
            default_value = "macos",
            converter = PlatformTypeConverter,
            documentation_category = UNDOCUMENTED,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION],
            help = concat!(
                "Don't set this value from the command line - it is derived from other flags and ",
                "configuration transitions derived from rule attributes",
            ),
        )]
        pub apple_platform_type: String,

        #[option(
            name = "apple_split_cpu",
            default_value = "",
            documentation_category = UNDOCUMENTED,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION],
            help = concat!(
                "Don't set this value from the command line - it is derived from other flags and ",
                "configuration transitions derived from rule attributes",
            ),
        )]
        pub apple_split_cpu: String,

        #[option(
            name = "apple configuration distinguisher",
            default_value = "UNKNOWN",
            converter = ConfigurationDistinguisherConverter,
            documentation_category = UNDOCUMENTED,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION],
            metadata_tags = [INTERNAL],
        )]
        pub configuration_distinguisher: AppleConfigurationConfigurationDistinguisher,

        #[option(
            name = "ios_multi_cpus",
            allow_multiple = true,
            converter = CommaSeparatedOptionListConverter,
            default_value = "null",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [LOSES_INCREMENTAL_STATE, LOADING_AND_ANALYSIS],
            help = concat!(
                "Comma-separated list of architectures to build an ios_application with. The result ",
                "is a universal binary containing all specified architectures.",
            ),
        )]
        pub ios_multi_cpus: Vec<String>,

        #[option(
            name = "visionos_cpus",
            allow_multiple = true,
            converter = CommaSeparatedOptionListConverter,
            default_value = "null",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [LOSES_INCREMENTAL_STATE, LOADING_AND_ANALYSIS],
            help = "Comma-separated list of architectures for which to build Apple visionOS binaries.",
        )]
        pub visionos_cpus: Vec<String>,

        #[option(
            name = "watchos_cpus",
            allow_multiple = true,
            converter = CommaSeparatedOptionListConverter,
            default_value = "null",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [LOSES_INCREMENTAL_STATE, LOADING_AND_ANALYSIS],
            help = "Comma-separated list of architectures for which to build Apple watchOS binaries.",
        )]
        pub watchos_cpus: Vec<String>,

        #[option(
            name = "tvos_cpus",
            allow_multiple = true,
            converter = CommaSeparatedOptionListConverter,
            default_value = "null",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [LOSES_INCREMENTAL_STATE, LOADING_AND_ANALYSIS],
            help = "Comma-separated list of architectures for which to build Apple tvOS binaries.",
        )]
        pub tvos_cpus: Vec<String>,

        #[option(
            name = "macos_cpus",
            allow_multiple = true,
            converter = CommaSeparatedOptionListConverter,
            default_value = "null",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [LOSES_INCREMENTAL_STATE, LOADING_AND_ANALYSIS],
            help = "Comma-separated list of architectures for which to build Apple macOS binaries.",
        )]
        pub macos_cpus: Vec<String>,

        #[option(
            name = "catalyst_cpus",
            allow_multiple = true,
            converter = CommaSeparatedOptionListConverter,
            default_value = "null",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [LOSES_INCREMENTAL_STATE, LOADING_AND_ANALYSIS],
            help = "Comma-separated list of architectures for which to build Apple Catalyst binaries.",
        )]
        pub catalyst_cpus: Vec<String>,

        #[option(
            name = "xcode_version_config",
            default_value = "@bazel_tools//tools/cpp:host_xcodes",
            converter = LabelConverter,
            documentation_category = TOOLCHAIN,
            effect_tags = [LOSES_INCREMENTAL_STATE, LOADING_AND_ANALYSIS],
            help = concat!(
                "The label of the xcode_config rule to be used for selecting the Xcode version ",
                "in the build configuration.",
            ),
        )]
        pub xcode_version_config: OptionLabel,

        #[option(
            name = "experimental_include_xcode_execution_requirements",
            default_value = "false",
            documentation_category = TOOLCHAIN,
            effect_tags = [LOSES_INCREMENTAL_STATE, LOADING_AND_ANALYSIS, EXECUTION],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "If set, add a \"requires-xcode:{version}\" execution requirement to every Xcode action.",
                "  If the Xcode version has a hyphenated label,  also add a",
                " \"requires-xcode-label:{version_label}\" execution requirement.",
            ),
        )]
        pub include_xcode_execution_requirements: bool,

        #[option(
            name = "incompatible_enable_apple_toolchain_resolution",
            default_value = "false",
            documentation_category = TOOLCHAIN,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "Use toolchain resolution to select the Apple SDK for apple rules (Starlark and native)",
        )]
        pub incompatible_use_toolchain_resolution: bool,

        #[option(
            name = "apple_platforms",
            converter = LabelListConverter,
            default_value = "",
            documentation_category = UNDOCUMENTED,
            effect_tags = [LOSES_INCREMENTAL_STATE, LOADING_AND_ANALYSIS],
            help = "Comma-separated list of platforms to use when building Apple binaries.",
        )]
        pub apple_platforms: Vec<OptionLabel>,

        #[option(
            name = "use_platforms_in_apple_crosstool_transition",
            default_value = "false",
            documentation_category = TOOLCHAIN,
            effect_tags = [LOADING_AND_ANALYSIS],
            help = concat!(
                "Makes apple_crosstool_transition fall back to using the value of `--platforms` flag",
                " instead of legacy `--cpu` when needed.",
            ),
        )]
        pub use_platforms_in_apple_crosstool_transition: bool,
    }
}

impl AppleCommandLineOptions {
    // Java: com/google/devtools/build/lib/rules/apple/AppleCommandLineOptions.java:358 AppleCommandLineOptions.getMinimumOsVersion
    /// Returns whether the minimum OS version is explicitly set for the current platform.
    pub fn get_minimum_os_version(&self) -> Option<DottedVersion> {
        let option: Option<DottedVersionOption> = match self.apple_platform_type.as_str() {
            platform_type::IOS | platform_type::CATALYST => self.ios_minimum_os.clone(),
            platform_type::MACOS => self.macos_minimum_os.clone(),
            platform_type::TVOS => self.tvos_minimum_os.clone(),
            platform_type::VISIONOS => {
                // TODO: Replace with CppOptions.minimumOsVersion
                DottedVersion::option(Some(DottedVersion::from_string_unchecked("1.0")))
            }
            platform_type::WATCHOS => self.watchos_minimum_os.clone(),
            _ => panic!("IllegalStateException"),
        };
        DottedVersion::maybe_unwrap(option.as_ref())
    }
}

impl FragmentOptions for AppleCommandLineOptions {}

// Java: com/google/devtools/build/lib/rules/apple/AppleCommandLineOptions.java:386 AppleCommandLineOptions.ConfigurationDistinguisherConverter
blaze_options::enum_converter!(
    pub ConfigurationDistinguisherConverter,
    AppleConfigurationConfigurationDistinguisher,
    "Apple rule configuration distinguisher"
);

// Java: com/google/devtools/build/lib/rules/apple/AppleCommandLineOptions.java:394 AppleCommandLineOptions.PlatformTypeConverter
/// Flag converter for PlatformType string flag, just converting to lowercase.
#[derive(Clone, Copy, Debug, Default)]
pub struct PlatformTypeConverter;

impl Converter for PlatformTypeConverter {
    // Java: com/google/devtools/build/lib/rules/apple/AppleCommandLineOptions.java:398 AppleCommandLineOptions.PlatformTypeConverter.convert
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        Ok(Some(Value::new(non_null(input).to_ascii_lowercase())))
    }
    fn get_type_description(&self) -> String {
        "a string".into()
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_util::{assert_defaults_round_trip, assert_option_names, parse};
    use super::*;

    #[test]
    fn option_names_match_java() {
        assert_option_names::<AppleCommandLineOptions>(&[
            "experimental_objc_provider_from_linked",
            "xcode_version",
            "ios_sdk_version",
            "watchos_sdk_version",
            "tvos_sdk_version",
            "macos_sdk_version",
            "ios_minimum_os",
            "watchos_minimum_os",
            "tvos_minimum_os",
            "macos_minimum_os",
            "host_macos_minimum_os",
            "experimental_prefer_mutual_xcode",
            "apple_platform_type",
            "apple_split_cpu",
            "apple configuration distinguisher",
            "ios_multi_cpus",
            "visionos_cpus",
            "watchos_cpus",
            "tvos_cpus",
            "macos_cpus",
            "catalyst_cpus",
            "xcode_version_config",
            "experimental_include_xcode_execution_requirements",
            "incompatible_enable_apple_toolchain_resolution",
            "apple_platforms",
            "use_platforms_in_apple_crosstool_transition",
        ]);
    }

    #[test]
    fn defaults_round_trip() {
        assert_defaults_round_trip::<AppleCommandLineOptions>(&[]);
    }

    #[test]
    fn spot_check_parses() {
        let o = parse::<AppleCommandLineOptions>(&[
            "--apple_platform_type=MACOS",
            "--macos_minimum_os=10.13",
            "--ios_multi_cpus=arm64,x86_64",
        ])
        .unwrap();
        assert_eq!(o.apple_platform_type, "macos");
        assert_eq!(
            o.get_minimum_os_version()
                .unwrap()
                .to_string_with_components(3),
            "10.13.0"
        );
        assert_eq!(o.ios_multi_cpus, vec!["arm64", "x86_64"]);
        assert_eq!(
            o.configuration_distinguisher,
            AppleConfigurationConfigurationDistinguisher::UNKNOWN
        );
    }
}
