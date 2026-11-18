//! Port of `analysis/PlatformOptions.java`.

use blaze_cmdline::Label;
use blaze_options::{
    CommaSeparatedOptionListConverter, ConversionContext, Converter, OptionsParsingException,
    Value, converter::non_null, options,
};

use super::converters::{ConvResult, LabelConverter, LabelListConverter, OptionLabel};
use super::fragment_options::FragmentOptions;
use super::platform_mapping_key::PlatformMappingKey;
use super::regex_filter::{RegexFilter, RegexFilterConverter};
use crate::misc::options_utils::PathFragmentConverter;

// Java: com/google/devtools/build/lib/analysis/PlatformOptions.java:43 PlatformOptions.DEFAULT_PLATFORM_NAMES
const DEFAULT_PLATFORM_NAMES: [&str; 5] = [
    "host",
    "host_platform",
    "target_platform",
    "default_host",
    "default_target",
];

// Java: com/google/devtools/build/lib/analysis/PlatformOptions.java:46 PlatformOptions.DEFAULT_HOST_PLATFORM
pub const DEFAULT_HOST_PLATFORM: &str = "@bazel_tools//tools:host_platform";

// Java: com/google/devtools/build/lib/analysis/PlatformOptions.java:41 PlatformOptions
options! {
    /// Command-line options for platform-related configuration.
    pub struct PlatformOptions {
        #[option(
            name = "host_platform",
            old_name = "experimental_host_platform",
            converter = HostPlatformConverter,
            default_value = "@bazel_tools//tools:host_platform",
            documentation_category = TOOLCHAIN,
            effect_tags = [AFFECTS_OUTPUTS, CHANGES_INPUTS, LOADING_AND_ANALYSIS],
            help = "The label of a platform rule that describes the host system.",
        )]
        pub host_platform: OptionLabel,

        #[option(
            name = "extra_execution_platforms",
            converter = CommaSeparatedOptionListConverter,
            default_value = "",
            documentation_category = TOOLCHAIN,
            effect_tags = [EXECUTION],
            help = concat!(
                "The platforms that are available as execution platforms to run actions.\n",
                "Platforms can be specified by exact target, or as a target pattern.\n",
                "These platforms will be considered before those declared in the `WORKSPACE` file by\n",
                "`register_execution_platforms()`. This option may only be set once; later\n",
                "instances will override earlier flag settings.\n",
            ),
        )]
        pub extra_execution_platforms: Vec<String>,

        #[option(
            name = "platforms",
            old_name = "experimental_platforms",
            converter = LabelListConverter,
            default_value = "",
            documentation_category = TOOLCHAIN,
            effect_tags = [AFFECTS_OUTPUTS, CHANGES_INPUTS, LOADING_AND_ANALYSIS],
            help = concat!(
                "The labels of the platform rules describing the target platforms for the current ",
                "command.",
            ),
        )]
        pub platforms: Vec<OptionLabel>,

        #[option(
            name = "extra_toolchains",
            default_value = "null",
            converter = CommaSeparatedOptionListConverter,
            documentation_category = TOOLCHAIN,
            allow_multiple = true,
            effect_tags = [AFFECTS_OUTPUTS, CHANGES_INPUTS, LOADING_AND_ANALYSIS],
            help = concat!(
                "The toolchain rules to be considered during toolchain resolution.\n",
                "Toolchains can be specified by exact target, or as a target pattern.\n",
                "These toolchains will be considered before those declared in the `WORKSPACE` file\n",
                "by `register_toolchains()`.\n",
            ),
        )]
        pub extra_toolchains: Vec<String>,

        #[option(
            name = "toolchain_resolution_debug",
            default_value = "-.*",
            converter = RegexFilterConverter,
            documentation_category = LOGGING,
            effect_tags = [TERMINAL_OUTPUT],
            help = concat!(
                "Print debug information during toolchain resolution. The flag takes a regex, which is",
                " checked against toolchain types and specific targets to see which to debug. ",
                "Multiple regexes may be  separated by commas, and then each regex is checked ",
                "separately. Note: The output of this flag is very complex and will likely only be ",
                "useful to experts in toolchain resolution.",
            ),
        )]
        pub toolchain_resolution_debug: RegexFilter,

        #[option(
            name = "incompatible_use_toolchain_resolution_for_java_rules",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [UNKNOWN],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "No-op. Kept here for backwards compatibility.",
        )]
        pub use_toolchain_resolution_for_java_rules: bool,

        #[option(
            name = "platform_mappings",
            converter = PlatformMappingKeyConverter,
            default_value = "",
            documentation_category = TOOLCHAIN,
            effect_tags = [AFFECTS_OUTPUTS, CHANGES_INPUTS, LOADING_AND_ANALYSIS],
            metadata_tags = [NON_CONFIGURABLE],
            help = concat!(
                "The location of a mapping file that describes which platform to use if none is set or\n",
                "which flags to set when a platform already exists. Must be relative to the main\n",
                "workspace root. Defaults to `platform_mappings` (a file directly under the\n",
                "workspace root).\n",
            ),
        )]
        pub platform_mapping_key: PlatformMappingKey,
    }
}

impl PlatformOptions {
    // Java: com/google/devtools/build/lib/analysis/PlatformOptions.java:48 PlatformOptions.platformIsDefault
    pub fn platform_is_default(platform: &Label) -> bool {
        DEFAULT_PLATFORM_NAMES.contains(&platform.get_name())
    }

    // Java: com/google/devtools/build/lib/analysis/PlatformOptions.java:169 PlatformOptions.dedupeKeepingLast
    /// Deduplicate the given list, keeping the last copy of any duplicates.
    ///
    /// Example: [a, b, a, c, b] -> [a, c, b]
    fn dedupe_keeping_last(values: Vec<String>) -> Vec<String> {
        // Check common cases.
        if values.len() <= 1 {
            return values;
        }
        // Reverse the list and then deduplicate.
        let mut reversed_result: Vec<String> = Vec::new();
        for v in values.iter().rev() {
            if !reversed_result.contains(v) {
                reversed_result.push(v.clone());
            }
        }
        // If there were no duplicates, return the exact same instance we got.
        if reversed_result.len() == values.len() {
            return values;
        }
        // Reverse the result to get back to the original order.
        reversed_result.reverse();
        reversed_result
    }

    // Java: com/google/devtools/build/lib/analysis/PlatformOptions.java:205 PlatformOptions.computeTargetPlatform
    /// Returns the intended target platform value based on options defined in this fragment.
    pub fn compute_target_platform(&self) -> &Label {
        match self.platforms.first() {
            Some(p) => p.label(),
            // Default to the host platform, whatever it is.
            None => self.host_platform.label(),
        }
    }
}

impl FragmentOptions for PlatformOptions {
    // Java: com/google/devtools/build/lib/analysis/PlatformOptions.java:189 PlatformOptions.getNormalized
    fn get_normalized(&self) -> PlatformOptions {
        let mut result = self.clone();
        result.extra_toolchains = Self::dedupe_keeping_last(result.extra_toolchains);
        // Only the first entry of platforms is used (it should have been Label and not
        // List<Label>) So drop all but the first entry.
        if result.platforms.len() > 1 {
            result.platforms.truncate(1);
        }
        result
    }
}

// Java: com/google/devtools/build/lib/analysis/PlatformOptions.java:218 PlatformOptions.HostPlatformConverter
/// Converter for `--host_platform` that returns the default host platform if the flag is set to
/// empty string.
#[derive(Clone, Copy, Debug, Default)]
struct HostPlatformConverter;

impl Converter for HostPlatformConverter {
    // Java: com/google/devtools/build/lib/analysis/PlatformOptions.java:221 PlatformOptions.HostPlatformConverter.convert
    fn convert(&self, input: Option<&str>, ctx: Option<&ConversionContext>) -> ConvResult {
        let input = non_null(input);
        if input.is_empty() {
            return LabelConverter.convert(Some(DEFAULT_HOST_PLATFORM), ctx);
        }
        LabelConverter.convert(Some(input), ctx)
    }
    fn get_type_description(&self) -> String {
        LabelConverter.get_type_description()
    }
}

// Java: com/google/devtools/build/lib/analysis/PlatformOptions.java:233 PlatformOptions.PlatformMappingKeyConverter
/// Converter for `--platform_mappings` that creates a canonical [`PlatformMappingKey`] for the
/// build.
#[derive(Clone, Copy, Debug, Default)]
struct PlatformMappingKeyConverter;

impl Converter for PlatformMappingKeyConverter {
    // Java: com/google/devtools/build/lib/analysis/PlatformOptions.java:237 PlatformOptions.PlatformMappingKeyConverter.convert
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        let input = non_null(input);
        if input.is_empty() {
            return Ok(Some(Value::new(PlatformMappingKey::default_key())));
        }
        let path = PathFragmentConverter.convert_path(input);
        if path.is_absolute() {
            return Err(OptionsParsingException::new(format!(
                "Expected relative path but got '{}'.",
                input
            )));
        }
        Ok(Some(Value::new(PlatformMappingKey::create_explicitly_set(
            path,
        ))))
    }
    // Java: com/google/devtools/build/lib/analysis/PlatformOptions.java:250 PlatformOptions.PlatformMappingKeyConverter.starlarkConvertible
    fn starlark_convertible(&self) -> bool {
        true
    }
    // Java: com/google/devtools/build/lib/analysis/PlatformOptions.java:255 PlatformOptions.PlatformMappingKeyConverter.reverseForStarlark
    fn reverse_for_starlark(&self, converted: &Value) -> String {
        let key = converted.get::<PlatformMappingKey>();
        if key == PlatformMappingKey::default_key() {
            String::new()
        } else {
            key.get_workspace_relative_mapping_path()
                .get_path_string()
                .to_string()
        }
    }
    fn get_type_description(&self) -> String {
        "a main workspace-relative path".into()
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_util::{
        assert_defaults_round_trip, assert_different, assert_option_names, assert_same,
        create_with_prefix, parse,
    };
    use super::*;
    use blaze_vfs::PathFragment;

    #[test]
    fn option_names_match_java() {
        assert_option_names::<PlatformOptions>(&[
            "host_platform",
            "extra_execution_platforms",
            "platforms",
            "extra_toolchains",
            "toolchain_resolution_debug",
            "incompatible_use_toolchain_resolution_for_java_rules",
            "platform_mappings",
        ]);
    }

    #[test]
    fn defaults_round_trip() {
        assert_defaults_round_trip::<PlatformOptions>(&[]);
    }

    #[test]
    fn spot_check_parses() {
        let o = parse::<PlatformOptions>(&[
            "--platforms=//p:a,//p:b",
            "--extra_toolchains=//t:1,//t:2",
            "--extra_toolchains=//t:3",
            "--toolchain_resolution_debug=foo",
        ])
        .unwrap();
        assert_eq!(o.compute_target_platform().to_string(), "//p:a");
        assert_eq!(o.extra_toolchains, vec!["//t:1", "//t:2", "//t:3"]);
        assert!(o.toolchain_resolution_debug.is_included("x/foo"));
        assert!(PlatformOptions::platform_is_default(
            &Label::parse_canonical("//x:host_platform").unwrap()
        ));
    }

    // Java: src/test/java/com/google/devtools/build/lib/analysis/PlatformOptionsTest.java
    const EXTRA_PLATFORMS_PREFIX: &str = "--extra_execution_platforms=";
    const EXTRA_TOOLCHAINS_PREFIX: &str = "--extra_toolchains=";
    const PLATFORMS_PREFIX: &str = "--platforms=";
    const PLATFORM_MAPPINGS_PREFIX: &str = "--platform_mappings=";
    const HOST_PLATFORM_PREFIX: &str = "--host_platform=";

    fn c(prefix: &str, args: &[&str]) -> PlatformOptions {
        create_with_prefix::<PlatformOptions>(prefix, args)
    }

    #[test]
    fn test_extra_platforms_order_matters() {
        // It seems that the platforms are considered in order. Picking the wrong one results in
        // broken builds. So add test asserting that order matters.
        assert_different(
            &c(EXTRA_PLATFORMS_PREFIX, &["platform1", "platform2"]),
            &c(EXTRA_PLATFORMS_PREFIX, &["platform2", "platform1"]),
        );
    }

    #[test]
    fn test_extra_toolchains_ordering() {
        // The ordering matters for tool chains, but the last one in the list has highest
        // priority.
        assert_different(
            &c(EXTRA_TOOLCHAINS_PREFIX, &["one", "two"]),
            &c(EXTRA_TOOLCHAINS_PREFIX, &["two", "one"]),
        );
    }

    #[test]
    fn test_extra_toolchains_duplicates() {
        // Specifying the same tool chain multiple times is a no-op.
        assert_same(
            &c(EXTRA_TOOLCHAINS_PREFIX, &["one", "one"]),
            &c(EXTRA_TOOLCHAINS_PREFIX, &["one"]),
        );
    }

    #[test]
    fn test_extra_toolchains_duplicates_keep_last() {
        // The last toolchain in the list has highest priority, so keep the last of any
        // duplicates.
        assert_same(
            &c(EXTRA_TOOLCHAINS_PREFIX, &["one", "two", "one"]),
            &c(EXTRA_TOOLCHAINS_PREFIX, &["two", "one"]),
        );
    }

    #[test]
    fn test_platforms_duplicates() {
        assert_same(
            &c(PLATFORMS_PREFIX, &["//p:one,//p:one"]),
            &c(PLATFORMS_PREFIX, &["//p:one"]),
        );
    }

    #[test]
    fn test_platforms_extra_values() {
        // Only the first value matters.
        assert_same(
            &c(PLATFORMS_PREFIX, &["//one,//two"]),
            &c(PLATFORMS_PREFIX, &["//one"]),
        );
    }

    #[test]
    fn test_platforms_order_matters() {
        // Changing the order changes the semantics.
        assert_different(
            &c(PLATFORMS_PREFIX, &["//one,//two"]),
            &c(PLATFORMS_PREFIX, &["//two,//one"]),
        );
    }

    #[test]
    fn platform_mappings_default() {
        let options = parse::<PlatformOptions>(&[]).unwrap();
        assert_eq!(
            options.platform_mapping_key,
            PlatformMappingKey::default_key()
        );
    }

    #[test]
    fn platform_mappings_custom() {
        let options = c(PLATFORM_MAPPINGS_PREFIX, &["a/b/platform_mappings"]);
        assert_eq!(
            options.platform_mapping_key,
            PlatformMappingKey::create_explicitly_set(PathFragment::create(
                "a/b/platform_mappings"
            ))
        );
    }

    #[test]
    fn platform_mappings_absolute_path_throws() {
        assert!(parse::<PlatformOptions>(&["--platform_mappings=/a/b/platform_mappings"]).is_err());
    }

    #[test]
    fn host_platform_empty_default() {
        let options = c(HOST_PLATFORM_PREFIX, &[""]);
        assert_eq!(
            options.host_platform,
            OptionLabel(Label::parse_canonical_unchecked(DEFAULT_HOST_PLATFORM))
        );
    }
}
