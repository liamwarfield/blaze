//! Port of `rules/config/FeatureFlagValue.java`: the values `config_feature_flag`s take in
//! `BuildOptions.getStarlarkOptions()`.
//!
//! PORT: Java's package is `rules.config`; the type lives here because `BuildOptions` stores it
//! (as [`StarlarkOptionValue::FeatureFlag`]).

use std::fmt;

use blaze_cmdline::Label;
use blaze_optdefs::fragment::config_feature_flag_options::ConfigFeatureFlagOptions;
use indexmap::{IndexMap, IndexSet};

use crate::build_options::{BuildOptions, StarlarkOptionValue};

/// Marker interface for detecting feature flags in the Starlark setting map.
// Java: com/google/devtools/build/lib/rules/config/FeatureFlagValue.java:30 FeatureFlagValue
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum FeatureFlagValue {
    /// A feature flag value for a flag known to be set to a particular value.
    // Java: com/google/devtools/build/lib/rules/config/FeatureFlagValue.java:33 FeatureFlagValue.SetValue
    SetValue(String),
    /// A feature flag value for a flag known to be set to its default value.
    // Java: com/google/devtools/build/lib/rules/config/FeatureFlagValue.java:49 FeatureFlagValue.DefaultValue
    DefaultValue,
    /// A feature flag value for a flag which was requested but which value was already trimmed.
    // Java: com/google/devtools/build/lib/rules/config/FeatureFlagValue.java:59 FeatureFlagValue.UnknownValue
    UnknownValue,
}

impl fmt::Display for FeatureFlagValue {
    // Java: com/google/devtools/build/lib/rules/config/FeatureFlagValue.java:43 FeatureFlagValue.SetValue.toString
    // Java: com/google/devtools/build/lib/rules/config/FeatureFlagValue.java:53 FeatureFlagValue.DefaultValue.toString
    // Java: com/google/devtools/build/lib/rules/config/FeatureFlagValue.java:63 FeatureFlagValue.UnknownValue.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FeatureFlagValue::SetValue(value) => write!(f, "FeatureFlagValue.SetValue{{{value}}}"),
            FeatureFlagValue::DefaultValue => write!(f, "FeatureFlagValue.DefaultValue{{}}"),
            FeatureFlagValue::UnknownValue => write!(f, "FeatureFlagValue.UnknownValue{{}}"),
        }
    }
}

impl FeatureFlagValue {
    // Java: com/google/devtools/build/lib/rules/config/FeatureFlagValue.java:39 FeatureFlagValue.SetValue.of
    pub fn set_value_of(value: impl Into<String>) -> FeatureFlagValue {
        FeatureFlagValue::SetValue(value.into())
    }

    /// Returns a new BuildOptions with a new map of feature flag values.
    // Java: com/google/devtools/build/lib/rules/config/FeatureFlagValue.java:69 FeatureFlagValue.replaceFlagValues
    pub fn replace_flag_values(
        original: &BuildOptions,
        new_values: &IndexMap<Label, String>,
    ) -> BuildOptions {
        let mut result = original.to_builder();
        for (key, value) in original.get_starlark_options() {
            if matches!(value, StarlarkOptionValue::FeatureFlag(_)) {
                result.remove_starlark_option(key);
            }
        }
        let mut new_value_objects: IndexMap<Label, StarlarkOptionValue> = IndexMap::new();
        for (key, value) in new_values {
            new_value_objects.insert(
                key.clone(),
                StarlarkOptionValue::FeatureFlag(FeatureFlagValue::set_value_of(value.clone())),
            );
        }
        result.add_starlark_options(new_value_objects);
        let mut built_result = result.build();
        if let Some(config_feature_flag_options) =
            built_result.get_mut::<ConfigFeatureFlagOptions>()
        {
            config_feature_flag_options.all_feature_flag_values_are_present = true;
        }
        built_result
    }

    /// Returns a new BuildOptions with the feature flag values trimmed down to the given flags.
    // Java: com/google/devtools/build/lib/rules/config/FeatureFlagValue.java:90 FeatureFlagValue.trimFlagValues
    pub fn trim_flag_values<'a>(
        original: &BuildOptions,
        available_flags: impl IntoIterator<Item = &'a Label>,
    ) -> BuildOptions {
        let available_flags: IndexSet<&Label> = available_flags.into_iter().collect();
        // An important performance property of this method is that we don't create a new
        // BuildOptions instance unless we really need one. This particularly saves the expensive
        // cost of BuildOptions.hashCode(). Since this method is called unconditionally over every
        // configured target, this has real observable effect on build analysis time.
        let mut seen_flags: IndexSet<&Label> = IndexSet::new();
        let mut flags_to_trim: IndexSet<&Label> = IndexSet::new();
        let mut unknown_flags_to_add: IndexMap<Label, FeatureFlagValue> = IndexMap::new();
        let original_config_feature_flag_options = original.get::<ConfigFeatureFlagOptions>();
        let change_all_values_present_option = original_config_feature_flag_options
            .is_some_and(|o| o.all_feature_flag_values_are_present);

        // What do we need to change?
        for (key, value) in original.get_starlark_options() {
            if matches!(value, StarlarkOptionValue::FeatureFlag(_)) {
                seen_flags.insert(key);
            }
        }
        // Sets.difference(seenFlags, availableFlags), in seenFlags' order.
        flags_to_trim.extend(
            seen_flags
                .iter()
                .filter(|f| !available_flags.contains(*f))
                .copied(),
        );
        let unknown_flag_value = if change_all_values_present_option {
            FeatureFlagValue::DefaultValue
        } else {
            FeatureFlagValue::UnknownValue
        };
        // Sets.difference(availableFlags, seenFlags), in availableFlags' order.
        for unknown_flag in available_flags.iter().filter(|f| !seen_flags.contains(*f)) {
            unknown_flags_to_add.insert((*unknown_flag).clone(), unknown_flag_value.clone());
        }

        // Nothing changed? Return the original BuildOptions.
        if flags_to_trim.is_empty()
            && unknown_flags_to_add.is_empty()
            && !change_all_values_present_option
        {
            return original.clone();
        }

        // Else construct a new one. This should not be the common case.
        let mut result = original.to_builder();
        for trimmed_flag in flags_to_trim {
            result.remove_starlark_option(trimmed_flag);
        }
        for (flag, value) in unknown_flags_to_add {
            result.add_starlark_option(flag, StarlarkOptionValue::FeatureFlag(value));
        }
        let mut built_result = result.build();
        if let Some(built_config_feature_flag_options) =
            built_result.get_mut::<ConfigFeatureFlagOptions>()
        {
            built_config_feature_flag_options.all_feature_flag_values_are_present = false;
        }
        built_result
    }
}
