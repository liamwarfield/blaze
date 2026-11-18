//! Option metadata enums. Variant names keep the Java spelling so that `@Option` annotations can
//! be transcribed token for token.

#![allow(non_camel_case_types)]

use crate::value::OptionScalar;

// Java: com/google/devtools/common/options/OptionEffectTag.java:55 OptionEffectTag
/// These tags describe the intent and effects of an option.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum OptionEffectTag {
    UNKNOWN,
    NO_OP,
    LOSES_INCREMENTAL_STATE,
    CHANGES_INPUTS,
    AFFECTS_OUTPUTS,
    BUILD_FILE_SEMANTICS,
    BAZEL_INTERNAL_CONFIGURATION,
    LOADING_AND_ANALYSIS,
    EXECUTION,
    HOST_MACHINE_RESOURCE_OPTIMIZATIONS,
    EAGERNESS_TO_EXIT,
    BAZEL_MONITORING,
    TERMINAL_OUTPUT,
    ACTION_COMMAND_LINES,
    TEST_RUNNER,
}

impl OptionEffectTag {
    pub const VALUES: &'static [OptionEffectTag] = &[
        Self::UNKNOWN,
        Self::NO_OP,
        Self::LOSES_INCREMENTAL_STATE,
        Self::CHANGES_INPUTS,
        Self::AFFECTS_OUTPUTS,
        Self::BUILD_FILE_SEMANTICS,
        Self::BAZEL_INTERNAL_CONFIGURATION,
        Self::LOADING_AND_ANALYSIS,
        Self::EXECUTION,
        Self::HOST_MACHINE_RESOURCE_OPTIMIZATIONS,
        Self::EAGERNESS_TO_EXIT,
        Self::BAZEL_MONITORING,
        Self::TERMINAL_OUTPUT,
        Self::ACTION_COMMAND_LINES,
        Self::TEST_RUNNER,
    ];

    // Java: com/google/devtools/common/options/OptionEffectTag.java:157 OptionEffectTag.getValue
    pub fn get_value(self) -> i32 {
        self as i32
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::UNKNOWN => "UNKNOWN",
            Self::NO_OP => "NO_OP",
            Self::LOSES_INCREMENTAL_STATE => "LOSES_INCREMENTAL_STATE",
            Self::CHANGES_INPUTS => "CHANGES_INPUTS",
            Self::AFFECTS_OUTPUTS => "AFFECTS_OUTPUTS",
            Self::BUILD_FILE_SEMANTICS => "BUILD_FILE_SEMANTICS",
            Self::BAZEL_INTERNAL_CONFIGURATION => "BAZEL_INTERNAL_CONFIGURATION",
            Self::LOADING_AND_ANALYSIS => "LOADING_AND_ANALYSIS",
            Self::EXECUTION => "EXECUTION",
            Self::HOST_MACHINE_RESOURCE_OPTIMIZATIONS => "HOST_MACHINE_RESOURCE_OPTIMIZATIONS",
            Self::EAGERNESS_TO_EXIT => "EAGERNESS_TO_EXIT",
            Self::BAZEL_MONITORING => "BAZEL_MONITORING",
            Self::TERMINAL_OUTPUT => "TERMINAL_OUTPUT",
            Self::ACTION_COMMAND_LINES => "ACTION_COMMAND_LINES",
            Self::TEST_RUNNER => "TEST_RUNNER",
        }
    }
}

// Java: com/google/devtools/common/options/OptionMetadataTag.java:20 OptionMetadataTag
/// Tags about the option itself, not its effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum OptionMetadataTag {
    EXPERIMENTAL,
    INCOMPATIBLE_CHANGE,
    DEPRECATED,
    HIDDEN,
    INTERNAL,
    NON_CONFIGURABLE,
}

impl OptionMetadataTag {
    pub const VALUES: &'static [OptionMetadataTag] = &[
        Self::EXPERIMENTAL,
        Self::INCOMPATIBLE_CHANGE,
        Self::DEPRECATED,
        Self::HIDDEN,
        Self::INTERNAL,
        Self::NON_CONFIGURABLE,
    ];

    // Java: com/google/devtools/common/options/OptionMetadataTag.java:76 OptionMetadataTag.getValue
    pub fn get_value(self) -> i32 {
        match self {
            Self::EXPERIMENTAL => 0,
            Self::INCOMPATIBLE_CHANGE => 1,
            Self::DEPRECATED => 2,
            Self::HIDDEN => 3,
            Self::INTERNAL => 4,
            Self::NON_CONFIGURABLE => 8,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::EXPERIMENTAL => "EXPERIMENTAL",
            Self::INCOMPATIBLE_CHANGE => "INCOMPATIBLE_CHANGE",
            Self::DEPRECATED => "DEPRECATED",
            Self::HIDDEN => "HIDDEN",
            Self::INTERNAL => "INTERNAL",
            Self::NON_CONFIGURABLE => "NON_CONFIGURABLE",
        }
    }
}

// Java: com/google/devtools/common/options/OptionDocumentationCategory.java:31 OptionDocumentationCategory
/// Categories used to group options in generated documentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum OptionDocumentationCategory {
    UNCATEGORIZED,
    UNDOCUMENTED,
    BAZEL_CLIENT_OPTIONS,
    LOGGING,
    INPUT_STRICTNESS,
    EXECUTION_STRATEGY,
    BUILD_TIME_OPTIMIZATION,
    OUTPUT_SELECTION,
    OUTPUT_PARAMETERS,
    SIGNING,
    STARLARK_SEMANTICS,
    TESTING,
    TOOLCHAIN,
    QUERY,
    MOD_COMMAND,
    BZLMOD,
    GENERIC_INPUTS,
    REMOTE,
}

impl OptionDocumentationCategory {
    pub fn name(self) -> &'static str {
        match self {
            Self::UNCATEGORIZED => "UNCATEGORIZED",
            Self::UNDOCUMENTED => "UNDOCUMENTED",
            Self::BAZEL_CLIENT_OPTIONS => "BAZEL_CLIENT_OPTIONS",
            Self::LOGGING => "LOGGING",
            Self::INPUT_STRICTNESS => "INPUT_STRICTNESS",
            Self::EXECUTION_STRATEGY => "EXECUTION_STRATEGY",
            Self::BUILD_TIME_OPTIMIZATION => "BUILD_TIME_OPTIMIZATION",
            Self::OUTPUT_SELECTION => "OUTPUT_SELECTION",
            Self::OUTPUT_PARAMETERS => "OUTPUT_PARAMETERS",
            Self::SIGNING => "SIGNING",
            Self::STARLARK_SEMANTICS => "STARLARK_SEMANTICS",
            Self::TESTING => "TESTING",
            Self::TOOLCHAIN => "TOOLCHAIN",
            Self::QUERY => "QUERY",
            Self::MOD_COMMAND => "MOD_COMMAND",
            Self::BZLMOD => "BZLMOD",
            Self::GENERIC_INPUTS => "GENERIC_INPUTS",
            Self::REMOTE => "REMOTE",
        }
    }
}

// Java: com/google/devtools/common/options/TriState.java:20 TriState
/// Enum used to represent tri-state options (yes/no/auto).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TriState {
    YES,
    NO,
    AUTO,
}

impl OptionScalar for TriState {
    fn java_to_string(&self) -> String {
        match self {
            TriState::YES => "YES",
            TriState::NO => "NO",
            TriState::AUTO => "AUTO",
        }
        .to_string()
    }
}
