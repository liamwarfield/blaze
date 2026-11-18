//! The options of `rules/proto/ProtoConfiguration.java` (`ProtoConfiguration.Options`). The fragment itself is not ported here.

use super::converters::{
    CoreOptionConvertersStrictDepsMode, EmptyToNullLabelConverter, LabelConverter,
    StrictDepsConverter,
};
use super::proto_constants;
use blaze_cmdline::Label;
use blaze_options::CommaSeparatedOptionSetConverter;
use blaze_options::Foreign;
use blaze_options::options;

use super::fragment_options::FragmentOptions;

// Java: com/google/devtools/build/lib/rules/proto/ProtoConfiguration.java:48 ProtoConfiguration.Options
options! {
    /// Command line options.
    pub struct ProtoConfigurationOptions {
        #[option(
            name = "protocopt",
            allow_multiple = true,
            default_value = "null",
            documentation_category = UNCATEGORIZED,
            effect_tags = [AFFECTS_OUTPUTS],
            help = "Additional options to pass to the protobuf compiler.",
        )]
        pub protoc_opts: Vec<String>,

        #[option(
            name = "experimental_proto_descriptor_sets_include_source_info",
            default_value = "false",
            documentation_category = OUTPUT_SELECTION,
            effect_tags = [AFFECTS_OUTPUTS, LOADING_AND_ANALYSIS],
            metadata_tags = [EXPERIMENTAL],
            help = "Run extra actions for alternative Java api versions in a proto_library.",
        )]
        pub experimental_proto_descriptor_sets_include_source_info: bool,

        #[option(
            name = "proto_compiler",
            default_value = proto_constants::DEFAULT_PROTOC_LABEL,
            converter = LabelConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [AFFECTS_OUTPUTS, LOADING_AND_ANALYSIS],
            help = "The label of the proto-compiler.",
        )]
        pub proto_compiler: Foreign<Label>,

        #[option(
            name = "proto_toolchain_for_javalite",
            default_value = proto_constants::DEFAULT_JAVA_LITE_PROTO_LABEL,
            converter = LabelConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [AFFECTS_OUTPUTS, LOADING_AND_ANALYSIS],
            help = "Label of proto_lang_toolchain() which describes how to compile JavaLite protos",
        )]
        pub proto_toolchain_for_java_lite: Foreign<Label>,

        #[option(
            name = "proto_toolchain_for_java",
            default_value = proto_constants::DEFAULT_JAVA_PROTO_LABEL,
            converter = EmptyToNullLabelConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [AFFECTS_OUTPUTS, LOADING_AND_ANALYSIS],
            help = "Label of proto_lang_toolchain() which describes how to compile Java protos",
        )]
        pub proto_toolchain_for_java: Option<Foreign<Label>>,

        #[option(
            name = "proto_toolchain_for_j2objc",
            default_value = proto_constants::DEFAULT_J2OBJC_PROTO_LABEL,
            category = "flags",
            converter = EmptyToNullLabelConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [AFFECTS_OUTPUTS, LOADING_AND_ANALYSIS],
            help = "Label of proto_lang_toolchain() which describes how to compile j2objc protos",
        )]
        pub proto_toolchain_for_j2objc: Option<Foreign<Label>>,

        #[option(
            name = "proto_toolchain_for_cc",
            default_value = proto_constants::DEFAULT_CC_PROTO_LABEL,
            converter = EmptyToNullLabelConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [AFFECTS_OUTPUTS, LOADING_AND_ANALYSIS],
            help = "Label of proto_lang_toolchain() which describes how to compile C++ protos",
        )]
        pub proto_toolchain_for_cc: Option<Foreign<Label>>,

        #[option(
            name = "strict_proto_deps",
            default_value = "error",
            converter = StrictDepsConverter,
            documentation_category = INPUT_STRICTNESS,
            effect_tags = [BUILD_FILE_SEMANTICS, EAGERNESS_TO_EXIT],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "Unless OFF, checks that a proto_library target explicitly declares all directly ",
                "used targets as dependencies.",
            ),
        )]
        pub strict_proto_deps: CoreOptionConvertersStrictDepsMode,

        #[option(
            name = "strict_public_imports",
            default_value = "off",
            converter = StrictDepsConverter,
            documentation_category = INPUT_STRICTNESS,
            effect_tags = [BUILD_FILE_SEMANTICS, EAGERNESS_TO_EXIT],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "Unless OFF, checks that a proto_library target explicitly declares all targets used ",
                "in 'import public' as exported.",
            ),
        )]
        pub strict_public_imports: CoreOptionConvertersStrictDepsMode,

        #[option(
            name = "cc_proto_library_header_suffixes",
            default_value = ".pb.h",
            documentation_category = OUTPUT_SELECTION,
            effect_tags = [AFFECTS_OUTPUTS, LOADING_AND_ANALYSIS],
            help = "Sets the suffixes of header files that a cc_proto_library creates.",
            converter = CommaSeparatedOptionSetConverter,
        )]
        pub cc_proto_library_header_suffixes: Vec<String>,

        #[option(
            name = "cc_proto_library_source_suffixes",
            default_value = ".pb.cc",
            documentation_category = OUTPUT_SELECTION,
            effect_tags = [AFFECTS_OUTPUTS, LOADING_AND_ANALYSIS],
            help = "Sets the suffixes of source files that a cc_proto_library creates.",
            converter = CommaSeparatedOptionSetConverter,
        )]
        pub cc_proto_library_source_suffixes: Vec<String>,
    }
}

impl FragmentOptions for ProtoConfigurationOptions {}

#[cfg(test)]
mod tests {
    use super::super::test_util::{assert_defaults_round_trip, assert_option_names, parse};
    use super::*;

    #[test]
    fn option_names_match_java() {
        assert_option_names::<ProtoConfigurationOptions>(&[
            "protocopt",
            "experimental_proto_descriptor_sets_include_source_info",
            "proto_compiler",
            "proto_toolchain_for_javalite",
            "proto_toolchain_for_java",
            "proto_toolchain_for_j2objc",
            "proto_toolchain_for_cc",
            "strict_proto_deps",
            "strict_public_imports",
            "cc_proto_library_header_suffixes",
            "cc_proto_library_source_suffixes",
        ]);
    }

    #[test]
    fn defaults_round_trip() {
        assert_defaults_round_trip::<ProtoConfigurationOptions>(&[]);
    }

    #[test]
    fn spot_check_parses() {
        let o = parse::<ProtoConfigurationOptions>(&[
            "--strict_proto_deps=error",
            "--proto_toolchain_for_cc=",
            "--protocopt=--a",
        ])
        .unwrap();
        assert_eq!(
            o.strict_proto_deps,
            CoreOptionConvertersStrictDepsMode::ERROR
        );
        assert!(o.proto_toolchain_for_cc.is_none());
        assert_eq!(o.protoc_opts, vec!["--a"]);
        assert_eq!(
            parse::<ProtoConfigurationOptions>(&[])
                .unwrap()
                .proto_compiler
                .to_string(),
            "@@bazel_tools//tools/proto:protoc"
        );
    }
}
