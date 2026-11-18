//! Port of `analysis/config/BuildOptionDetails.java`.

use blaze_build_config::build_options::{BuildOptions, StarlarkOptionValue};
use blaze_build_config::fragment_options_class::FragmentOptionsClass;
use blaze_cmdline::Label;
use blaze_options::OptionMetadataTag;
use blaze_options::Value as OptionValue;
use indexmap::IndexMap;

/// Introspector for option details, to be used when `--define` and `--copt` don't cut it
/// (Java `BuildOptionDetails`: maps option names to their values and definitions).
// Java: com/google/devtools/build/lib/analysis/config/BuildOptionDetails.java:37 BuildOptionDetails
#[derive(Debug)]
pub struct BuildOptionDetails {
    /// Maps native option names to the details the option takes for this configuration.
    native_options_map: IndexMap<String, OptionDetails>,
    /// For options with `@Option.oldName`, maps the old name to the canonical name.
    old_name_to_canonical_name: IndexMap<String, String>,
    /// Maps Starlark option labels to values.
    starlark_options_map: IndexMap<Label, StarlarkOptionValue>,
}

// Java: com/google/devtools/build/lib/analysis/config/BuildOptionDetails.java:76 BuildOptionDetails.OptionDetails
#[derive(Debug)]
struct OptionDetails {
    /// The `FragmentOptions` class that defines this option.
    options_class: FragmentOptionsClass,
    /// The value of the given option (either explicitly defined or default). May be null.
    value: Option<OptionValue>,
    /// Whether or not this option supports multiple values.
    allows_multiple: bool,
}

impl BuildOptionDetails {
    /// Builds a `BuildOptionDetails` for the given set of native and Starlark options
    /// (Java `forOptions(buildOptions.getNativeOptions(), buildOptions.getStarlarkOptions())`, as
    /// `BuildConfigurationValue.getBuildOptionDetails` calls it).
    // Java: com/google/devtools/build/lib/analysis/config/BuildOptionDetails.java:47 BuildOptionDetails.forOptions
    pub fn for_options(build_options: &BuildOptions) -> BuildOptionDetails {
        let mut map: IndexMap<String, OptionDetails> = IndexMap::new();
        let mut old_name_to_canonical_name: IndexMap<String, String> = IndexMap::new();
        for (options_class, options) in build_options.get_native_options() {
            for option_definition in options_class.get_option_definitions() {
                if option_definition
                    .get_option_metadata_tags()
                    .contains(&OptionMetadataTag::INTERNAL)
                {
                    // ignore internal options
                    continue;
                }
                if !option_definition.get_old_option_name().is_empty() {
                    put_or_throw(
                        &mut old_name_to_canonical_name,
                        option_definition.get_old_option_name().to_owned(),
                        option_definition.get_option_name().to_owned(),
                    );
                }
                let value = options.get_value_from_definition(option_definition);
                put_or_throw(
                    &mut map,
                    option_definition.get_option_name().to_owned(),
                    OptionDetails {
                        options_class,
                        value,
                        allows_multiple: option_definition.allows_multiple(),
                    },
                );
            }
        }
        BuildOptionDetails {
            native_options_map: map,
            old_name_to_canonical_name,
            starlark_options_map: build_options.get_starlark_options().clone(),
        }
    }

    /// Returns the `FragmentOptions` class that defines the given option, `None` if the option
    /// isn't recognized.
    // Java: com/google/devtools/build/lib/analysis/config/BuildOptionDetails.java:131 BuildOptionDetails.getOptionClass
    pub fn get_option_class(&self, option_name: &str) -> Option<FragmentOptionsClass> {
        self.native_options_map
            .get(option_name)
            .map(|d| d.options_class)
    }

    /// Returns the value of the specified native option for this configuration or `None` if the
    /// option isn't recognized (or its value is null).
    // Java: com/google/devtools/build/lib/analysis/config/BuildOptionDetails.java:145 BuildOptionDetails.getOptionValue
    pub fn get_option_value(&self, option_name: &str) -> Option<&OptionValue> {
        self.native_options_map
            .get(option_name)
            .and_then(|d| d.value.as_ref())
    }

    /// Returns the value of the specified Starlark option or `None` if it isn't recognized.
    // Java: com/google/devtools/build/lib/analysis/config/BuildOptionDetails.java:152 BuildOptionDetails.getOptionValue
    pub fn get_starlark_option_value(&self, option_name: &Label) -> Option<&StarlarkOptionValue> {
        self.starlark_options_map.get(option_name)
    }

    /// If this is an `oldName` alias for a canonical option name, returns the canonical name.
    /// Else returns the original name (since there's only one).
    // Java: com/google/devtools/build/lib/analysis/config/BuildOptionDetails.java:160 BuildOptionDetails.getCanonicalName
    pub fn get_canonical_name<'a>(&'a self, option_name: &'a str) -> &'a str {
        self.old_name_to_canonical_name
            .get(option_name)
            .map(String::as_str)
            .unwrap_or(option_name)
    }

    /// Returns whether or not the given option supports multiple values at the command line.
    /// Returns false for unrecognized options.
    // Java: com/google/devtools/build/lib/analysis/config/BuildOptionDetails.java:173 BuildOptionDetails.allowsMultipleValues
    pub fn allows_multiple_values(&self, option_name: &str) -> bool {
        self.native_options_map
            .get(option_name)
            .is_some_and(|d| d.allows_multiple)
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildOptionDetails.java:178 BuildOptionDetails.isNonConfigurable
    pub fn is_non_configurable(&self, option_name: &str) -> bool {
        let Some(option_details) = self.native_options_map.get(option_name) else {
            return false;
        };
        let Some(option_definition) = option_details
            .options_class
            .get_option_definitions()
            .find(|d| d.get_option_name() == option_name)
        else {
            return false;
        };
        option_definition
            .get_option_metadata_tags()
            .contains(&OptionMetadataTag::NON_CONFIGURABLE)
    }
}

/// `ImmutableMap.Builder.put` + `buildOrThrow`: duplicate keys are an error.
fn put_or_throw<V>(map: &mut IndexMap<String, V>, key: String, value: V) {
    if map.contains_key(&key) {
        panic!("IllegalArgumentException: Multiple entries with same key: {key}");
    }
    map.insert(key, value);
}
