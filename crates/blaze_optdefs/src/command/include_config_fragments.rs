//! `CoreOptions.IncludeConfigFragmentsEnum` and its converter, from
//! `com/google/devtools/build/lib/analysis/config/CoreOptions.java` (used by `cquery --show_config_fragments`).
//!
//! PORT: `CoreOptions` itself is transcribed in `crate::fragment`; this copy of the enum exists
//! because the two modules are written separately. Unify when both are in.

use super::prelude::*;

// Java: com/google/devtools/build/lib/analysis/config/CoreOptions.java:1173 CoreOptions.IncludeConfigFragmentsEnum
/// Values for the --include_config_fragments_provider option.
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum IncludeConfigFragmentsEnum {
    /// Don't offer the provider at all. This is best for most builds, which don't use this
    /// information and don't need the extra memory hit over every configured target.
    OFF,
    /// Provide the fragments required *directly* by this rule.
    DIRECT,
    /// Provide the fragments required by this rule and its transitive dependencies.
    TRANSITIVE,
}

impl OptionScalar for IncludeConfigFragmentsEnum {
    fn java_to_string(&self) -> String {
        self.name().to_string()
    }
}

impl blaze_options::converters::JavaEnum for IncludeConfigFragmentsEnum {
    fn values() -> &'static [Self] {
        &[
            IncludeConfigFragmentsEnum::OFF,
            IncludeConfigFragmentsEnum::DIRECT,
            IncludeConfigFragmentsEnum::TRANSITIVE,
        ]
    }
    fn name(&self) -> &'static str {
        match self {
            IncludeConfigFragmentsEnum::OFF => "OFF",
            IncludeConfigFragmentsEnum::DIRECT => "DIRECT",
            IncludeConfigFragmentsEnum::TRANSITIVE => "TRANSITIVE",
        }
    }
    // The Java enum implements StarlarkValue.
    fn is_starlark_value() -> bool {
        true
    }
}

option_field_type!(
    IncludeConfigFragmentsEnum,
    "com.google.devtools.build.lib.analysis.config.CoreOptions$IncludeConfigFragmentsEnum",
    raw = IncludeConfigFragmentsEnum::OFF
);

// Java: com/google/devtools/build/lib/analysis/config/CoreOptions.java:1186 CoreOptions.IncludeConfigFragmentsEnumConverter
enum_converter!(
    pub IncludeConfigFragmentsEnumConverter,
    IncludeConfigFragmentsEnum,
    "include config fragments provider option"
);
