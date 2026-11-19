//! Port of `OptionFilterDescriptions`: descriptions of the options filters, for use in generated
//! documentation and usage text.

use indexmap::IndexMap;

use crate::tags::{OptionDocumentationCategory, OptionEffectTag, OptionMetadataTag};

/// Provides descriptions of the options filters, for use in generated documentation and usage
/// text.
// Java: com/google/devtools/common/options/OptionFilterDescriptions.java:21 OptionFilterDescriptions
pub struct OptionFilterDescriptions;

impl OptionFilterDescriptions {
    /// The order that the categories should be listed in.
    // Java: com/google/devtools/common/options/OptionFilterDescriptions.java:24 OptionFilterDescriptions.documentationOrder
    pub const DOCUMENTATION_ORDER: &'static [OptionDocumentationCategory] = &[
        OptionDocumentationCategory::BAZEL_CLIENT_OPTIONS,
        OptionDocumentationCategory::EXECUTION_STRATEGY,
        OptionDocumentationCategory::TOOLCHAIN,
        OptionDocumentationCategory::OUTPUT_SELECTION,
        OptionDocumentationCategory::OUTPUT_PARAMETERS,
        OptionDocumentationCategory::INPUT_STRICTNESS,
        OptionDocumentationCategory::SIGNING,
        OptionDocumentationCategory::STARLARK_SEMANTICS,
        OptionDocumentationCategory::TESTING,
        OptionDocumentationCategory::QUERY,
        OptionDocumentationCategory::MOD_COMMAND,
        OptionDocumentationCategory::BZLMOD,
        OptionDocumentationCategory::BUILD_TIME_OPTIMIZATION,
        OptionDocumentationCategory::LOGGING,
        OptionDocumentationCategory::GENERIC_INPUTS,
        OptionDocumentationCategory::REMOTE,
        OptionDocumentationCategory::UNCATEGORIZED,
    ];

    // Java: com/google/devtools/common/options/OptionFilterDescriptions.java:44 OptionFilterDescriptions.getOptionCategoriesEnumDescription
    pub fn get_option_categories_enum_description() -> IndexMap<OptionDocumentationCategory, String>
    {
        use OptionDocumentationCategory::*;
        let mut m = IndexMap::new();
        m.insert(
            UNCATEGORIZED,
            "Miscellaneous options, not otherwise categorized.".to_string(),
        );
        // Here for completeness, the help output should not include this option.
        m.insert(
            UNDOCUMENTED,
            "This feature should not be documented, as it is not meant for general use".to_string(),
        );
        m.insert(
            BAZEL_CLIENT_OPTIONS,
            "Options that appear before the command and are parsed by the client".to_string(),
        );
        m.insert(
            LOGGING,
            "Options that affect the verbosity, format or location of logging".to_string(),
        );
        m.insert(
            EXECUTION_STRATEGY,
            "Options that control build execution".to_string(),
        );
        m.insert(
            BUILD_TIME_OPTIMIZATION,
            "Options that trigger optimizations of the build time".to_string(),
        );
        m.insert(
            OUTPUT_SELECTION,
            "Options that control the output of the command".to_string(),
        );
        m.insert(
            OUTPUT_PARAMETERS,
            concat!(
                "Options that let the user configure the intended output, affecting its value, as ",
                "opposed to its existence"
            )
            .to_string(),
        );
        m.insert(
            INPUT_STRICTNESS,
            concat!(
                "Options that affect how strictly Bazel enforces valid build inputs (rule ",
                "definitions, ",
                " flag combinations, etc.)"
            )
            .to_string(),
        );
        m.insert(
            SIGNING,
            "Options that affect the signing outputs of a build".to_string(),
        );
        m.insert(
            STARLARK_SEMANTICS,
            concat!(
                "This option affects semantics of the Starlark language or the build API ",
                "accessible to ",
                "BUILD files, .bzl files, or WORKSPACE files."
            )
            .to_string(),
        );
        m.insert(
            TESTING,
            "Options that govern the behavior of the test environment or test runner".to_string(),
        );
        m.insert(
            TOOLCHAIN,
            "Options that configure the toolchain used for action execution".to_string(),
        );
        m.insert(
            QUERY,
            "Options relating to query output and semantics".to_string(),
        );
        m.insert(
            MOD_COMMAND,
            "Options relating to the output and semantics of the `mod` subcommand".to_string(),
        );
        m.insert(
            BZLMOD,
            "Options relating to Bzlmod output and semantics".to_string(),
        );
        m.insert(
            GENERIC_INPUTS,
            concat!(
                "Options specifying or altering a generic input to a Bazel command that does not ",
                "fall ",
                "into other categories."
            )
            .to_string(),
        );
        m.insert(REMOTE, "Remote caching and execution options".to_string());
        m
    }

    // Java: com/google/devtools/common/options/OptionFilterDescriptions.java:104 OptionFilterDescriptions.getOptionEffectTagDescription
    pub fn get_option_effect_tag_description(
        product_name: &str,
    ) -> IndexMap<OptionEffectTag, String> {
        use OptionEffectTag::*;
        let mut m = IndexMap::new();
        m.insert(
            UNKNOWN,
            "This option has unknown, or undocumented, effect.".to_string(),
        );
        m.insert(NO_OP, "This option has literally no effect.".to_string());
        m.insert(
            LOSES_INCREMENTAL_STATE,
            concat!(
                "Changing the value of this option can cause significant loss of incremental ",
                "state, which slows builds. State could be lost due to a server restart or to ",
                "invalidation of a large part of the dependency graph."
            )
            .to_string(),
        );
        m.insert(
            CHANGES_INPUTS,
            format!(
                "This option actively changes the inputs that {} considers for the build, such \
                 as filesystem restrictions, repository versions, or other options.",
                product_name
            ),
        );
        m.insert(
            AFFECTS_OUTPUTS,
            format!(
                "This option affects {}'s outputs. This tag is intentionally broad, can include \
                 transitive affects, and does not specify the type of output it affects.",
                product_name
            ),
        );
        m.insert(
            BUILD_FILE_SEMANTICS,
            "This option affects the semantics of BUILD or .bzl files.".to_string(),
        );
        m.insert(
            BAZEL_INTERNAL_CONFIGURATION,
            format!(
                "This option affects settings of {}-internal machinery. This tag does not, on \
                 its own, mean that build artifacts are affected.",
                product_name
            ),
        );
        m.insert(
            LOADING_AND_ANALYSIS,
            concat!(
                "This option affects the loading and analysis of dependencies, and the building ",
                "of the dependency graph."
            )
            .to_string(),
        );
        m.insert(
            EXECUTION,
            concat!(
                "This option affects the execution phase, such as sandboxing or remote execution ",
                "related options."
            )
            .to_string(),
        );
        m.insert(
            HOST_MACHINE_RESOURCE_OPTIMIZATIONS,
            concat!(
                "This option triggers an optimization that may be machine specific and is not ",
                "guaranteed to work on all machines. The optimization could include a tradeoff ",
                "with other aspects of performance, such as memory or cpu cost."
            )
            .to_string(),
        );
        m.insert(
            EAGERNESS_TO_EXIT,
            format!(
                "This option changes how eagerly {} will exit from a failure, where a choice \
                 between continuing despite the failure and ending the invocation exists.",
                product_name
            ),
        );
        m.insert(
            BAZEL_MONITORING,
            format!(
                "This option is used to monitor {}'s behavior and performance.",
                product_name
            ),
        );
        m.insert(
            TERMINAL_OUTPUT,
            format!("This option affects {}'s terminal output.", product_name),
        );
        m.insert(
            ACTION_COMMAND_LINES,
            "This option changes the command line arguments of one or more build actions."
                .to_string(),
        );
        m.insert(
            TEST_RUNNER,
            "This option changes the testrunner environment of the build.".to_string(),
        );
        m
    }

    // Java: com/google/devtools/common/options/OptionFilterDescriptions.java:170 OptionFilterDescriptions.getOptionMetadataTagDescription
    pub fn get_option_metadata_tag_description(
        _product_name: &str,
    ) -> IndexMap<OptionMetadataTag, String> {
        use OptionMetadataTag::*;
        let mut m = IndexMap::new();
        m.insert(
            EXPERIMENTAL,
            "This option triggers an experimental feature with no guarantees of functionality."
                .to_string(),
        );
        m.insert(
            INCOMPATIBLE_CHANGE,
            concat!(
                "This option triggers a breaking change. Use this option to test your migration ",
                "readiness or get early access to the new feature"
            )
            .to_string(),
        );
        m.insert(
            DEPRECATED,
            concat!(
                "This option is deprecated. It might be that the feature it affects is ",
                "deprecated, or that another method of supplying the information is preferred."
            )
            .to_string(),
        );
        // Here for completeness, these options are UNDOCUMENTED.
        m.insert(
            HIDDEN,
            "This option should not be used by a user, and should not be logged.".to_string(),
        );
        // Here for completeness, these options are UNDOCUMENTED.
        m.insert(
            INTERNAL,
            "This option isn't even a option, and should not be logged.".to_string(),
        );
        m.insert(
            NON_CONFIGURABLE,
            "This option cannot be changed in a transition or be used in a select() statement."
                .to_string(),
        );
        m
    }
}
