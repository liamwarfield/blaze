//! Ports of `skyframe/config/ParsedFlagsValue.java` and `skyframe/config/NativeAndStarlarkFlags.java`:
//! a list of command-line style flags (`--compilation_mode=dbg`, `--//custom/starlark:flag=foo`)
//! parsed into native and Starlark option values, and the merge of those values into a
//! `BuildOptions` (platform-based flags, platform mappings).
//!
//! The function computing a [`ParsedFlagsKey`] (`ParsedFlagsFunction`) needs package loading and
//! the Starlark options parser; it lives in `blaze_sky_analysis::parsed_flags_function`.

use std::collections::BTreeMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use blaze_cmdline::{Label, LabelPackageContext, RepositoryMapping};
use blaze_options::{OptionsParser, OptionsParsingException, Value};
use blaze_skyframe::{FunctionHermeticity, Key, SkyFunctionName};

use crate::build_configuration_key::BuildConfigurationKey;
use crate::build_options::{BuildOptions, BuildOptionsBuilder, StarlarkOptionValue};
use crate::fragment_options_class::FragmentOptionsClass;

// Java: com/google/devtools/build/lib/skyframe/config/ParsedFlagsValue.java:47 ParsedFlagsValue.Key
/// Key for [`ParsedFlagsValue`] based on the raw flags.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ParsedFlagsKey {
    raw_flags: Vec<String>,
    package_context: LabelPackageContext,
    include_default_values: bool,
    /// `ImmutableMap<String, Label>` (insertion order).
    flag_alias_mappings: Vec<(String, Label)>,
}

impl ParsedFlagsKey {
    // Java: com/google/devtools/build/lib/skyframe/config/ParsedFlagsValue.java:69 ParsedFlagsValue.Key.create
    /// Returns a new key for the given command-line flags, such as `--compilation_mode=bdg` or
    /// `--//custom/starlark:flag=23`.
    pub fn create(
        raw_flags: Vec<String>,
        package_context: LabelPackageContext,
        flag_alias_mappings: Vec<(String, Label)>,
    ) -> ParsedFlagsKey {
        Self::create_with_defaults(raw_flags, package_context, false, flag_alias_mappings)
    }

    // Java: com/google/devtools/build/lib/skyframe/config/ParsedFlagsValue.java:81 ParsedFlagsValue.Key.create
    pub fn create_with_defaults(
        raw_flags: Vec<String>,
        package_context: LabelPackageContext,
        include_default_values: bool,
        flag_alias_mappings: Vec<(String, Label)>,
    ) -> ParsedFlagsKey {
        ParsedFlagsKey {
            raw_flags,
            package_context,
            include_default_values,
            flag_alias_mappings,
        }
    }

    pub fn raw_flags(&self) -> &[String] {
        &self.raw_flags
    }

    pub fn package_context(&self) -> &LabelPackageContext {
        &self.package_context
    }

    pub fn include_default_values(&self) -> bool {
        self.include_default_values
    }

    pub fn flag_alias_mappings(&self) -> &[(String, Label)] {
        &self.flag_alias_mappings
    }
}

impl fmt::Debug for ParsedFlagsKey {
    // Java: the record's toString.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Key[rawFlags=[{}], packageContext={:?}, includeDefaultValues={}, flagAliasMappings={{{}}}]",
            self.raw_flags.join(", "),
            self.package_context,
            self.include_default_values,
            self.flag_alias_mappings
                .iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

impl Key for ParsedFlagsKey {
    type Value = ParsedFlagsValue;
    const FUNCTION: SkyFunctionName = SkyFunctionName("PARSED_FLAGS");
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:114 SkyFunctions.PARSED_FLAGS (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    /// Java: the flags only (`NativeAndStarlarkFlags`, an AutoValue), not the parsing result. PORT: the options classes are compared in order (Java: a set); they come from the server's fragment registry, so only a reordering differs, and then Rust does not prune.
    // Java: com/google/devtools/build/lib/skyframe/config/ParsedFlagsValue.java:226 ParsedFlagsValue.equals
    fn values_equal(old: &ParsedFlagsValue, new: &ParsedFlagsValue) -> bool {
        old == new
    }
}

// Java: com/google/devtools/build/lib/skyframe/config/NativeAndStarlarkFlags.java:33 NativeAndStarlarkFlags
/// Container for storing a set of native and Starlark flag settings in separate buckets.
///
/// This is necessary because native and Starlark flags are parsed with different logic.
// Java: com/google/devtools/build/lib/skyframe/config/NativeAndStarlarkFlags.java:57 NativeAndStarlarkFlags.builder
// PORT: the AutoValue builder is the struct literal (`..Default::default()`); its setters are the
// fields.
// Java: com/google/devtools/build/lib/skyframe/config/NativeAndStarlarkFlags.java:53 NativeAndStarlarkFlags.Builder.build
#[derive(Clone, Default)]
pub struct NativeAndStarlarkFlags {
    /// The native flags from a given set of flags, in the format `[--flag=value]` or
    /// `["--flag", "value"]`.
    // Java: com/google/devtools/build/lib/skyframe/config/NativeAndStarlarkFlags.java:40 NativeAndStarlarkFlags.Builder.nativeFlags
    pub native_flags: Vec<String>,
    /// The Starlark flags from a given set of flags, mapped to the correct converted data type.
    /// If a Starlark flag is explicitly set to the default value it should still appear in this
    /// map so that consumers can properly handle the flag.
    // Java: com/google/devtools/build/lib/skyframe/config/NativeAndStarlarkFlags.java:42 NativeAndStarlarkFlags.Builder.starlarkFlags
    pub starlark_flags: BTreeMap<String, StarlarkOptionValue>,
    // Java: com/google/devtools/build/lib/skyframe/config/NativeAndStarlarkFlags.java:44 NativeAndStarlarkFlags.Builder.scopesAttributes
    pub scopes_attributes: BTreeMap<String, String>,
    // TODO: https://github.com/bazelbuild/bazel/issues/22365 - Improve looking up Starlark flag
    // option definitions and do not store this.
    // Java: com/google/devtools/build/lib/skyframe/config/NativeAndStarlarkFlags.java:46 NativeAndStarlarkFlags.Builder.starlarkFlagDefaults
    pub starlark_flag_defaults: BTreeMap<String, StarlarkOptionValue>,
    // Java: com/google/devtools/build/lib/skyframe/config/NativeAndStarlarkFlags.java:48 NativeAndStarlarkFlags.Builder.optionsClasses
    pub options_classes: Vec<FragmentOptionsClass>,
    // Java: com/google/devtools/build/lib/skyframe/config/NativeAndStarlarkFlags.java:51 NativeAndStarlarkFlags.Builder.repoMapping
    pub repo_mapping: Option<RepositoryMapping>,
}

impl NativeAndStarlarkFlags {
    // Java: com/google/devtools/build/lib/skyframe/config/NativeAndStarlarkFlags.java:90 NativeAndStarlarkFlags.parse
    pub fn parse(&self) -> Result<OptionsParser, OptionsParsingException> {
        let classes: Vec<_> = self
            .options_classes
            .iter()
            .map(|c| c.options_class())
            .collect();
        let mut builder = OptionsParser::builder()
            .options_classes(&classes)
            // We need the ability to re-map internal options in the mappings file.
            .ignore_internal_options(false);
        if let Some(repo_mapping) = &self.repo_mapping {
            builder = builder.with_conversion_context(Arc::new(repo_mapping.clone()));
        }
        let mut parser = builder
            .build()
            .unwrap_or_else(|e| panic!("{}", e.get_message()));
        parser.parse_list(self.native_flags.clone())?;
        parser.set_starlark_options(
            self.starlark_flags
                .iter()
                .map(|(name, value)| (name.clone(), Value::new(value.clone())))
                .collect(),
        );
        parser.set_scopes_attributes(self.scopes_attributes.clone());
        Ok(parser)
    }
}

/// Java `Map.equals` of two `ImmutableMap<String, Object>` of Starlark values.
fn starlark_maps_equal(
    a: &BTreeMap<String, StarlarkOptionValue>,
    b: &BTreeMap<String, StarlarkOptionValue>,
) -> bool {
    a.len() == b.len()
        && a.iter()
            .all(|(k, v)| b.get(k).is_some_and(|other| v.java_equals(other)))
}

impl PartialEq for NativeAndStarlarkFlags {
    // Java: the AutoValue equals.
    fn eq(&self, other: &Self) -> bool {
        self.native_flags == other.native_flags
            && starlark_maps_equal(&self.starlark_flags, &other.starlark_flags)
            && self.scopes_attributes == other.scopes_attributes
            && starlark_maps_equal(&self.starlark_flag_defaults, &other.starlark_flag_defaults)
            && self.options_classes.len() == other.options_classes.len()
            && self
                .options_classes
                .iter()
                .zip(&other.options_classes)
                .all(|(a, b)| std::ptr::eq(*a, *b))
            && self.repo_mapping == other.repo_mapping
    }
}

impl Eq for NativeAndStarlarkFlags {}

impl Hash for NativeAndStarlarkFlags {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Values without a Rust `Hash` (Starlark values) are left out; equal values still hash
        // equally.
        self.native_flags.hash(state);
        for k in self.starlark_flags.keys() {
            k.hash(state);
        }
        self.scopes_attributes.hash(state);
    }
}

impl fmt::Debug for NativeAndStarlarkFlags {
    // Java: the AutoValue toString.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "NativeAndStarlarkFlags{{nativeFlags=[{}], starlarkFlags={{{}}}, scopesAttributes={:?}}}",
            self.native_flags.join(", "),
            self.starlark_flags
                .iter()
                .map(|(k, v)| format!("{}={}", k, v.java_to_string()))
                .collect::<Vec<_>>()
                .join(", "),
            self.scopes_attributes
        )
    }
}

// Java: com/google/devtools/build/lib/skyframe/config/ParsedFlagsValue.java:42 ParsedFlagsValue
/// Stores the `OptionsParsingResult` from `ParsedFlagsFunction`.
pub struct ParsedFlagsValue {
    flags: NativeAndStarlarkFlags,
    parsing_result: OptionsParser,
}

impl ParsedFlagsValue {
    // Java: com/google/devtools/build/lib/skyframe/config/ParsedFlagsValue.java:101 ParsedFlagsValue.parseAndCreate
    pub fn parse_and_create(
        flags: NativeAndStarlarkFlags,
    ) -> Result<ParsedFlagsValue, OptionsParsingException> {
        let parsing_result = flags.parse()?;
        Ok(ParsedFlagsValue {
            flags,
            parsing_result,
        })
    }

    // Java: com/google/devtools/build/lib/skyframe/config/ParsedFlagsValue.java:127 ParsedFlagsValue.parsingResult
    pub fn parsing_result(&self) -> &OptionsParser {
        &self.parsing_result
    }

    pub fn flags(&self) -> &NativeAndStarlarkFlags {
        &self.flags
    }

    // Java: com/google/devtools/build/lib/skyframe/config/ParsedFlagsValue.java:161 ParsedFlagsValue.mergeWith
    /// Returns a new `BuildConfigurationKey` with options containing all flags from the given
    /// `BuildOptions` with `parsing_result()` merged in.
    ///
    /// The merging logic is as follows:
    ///
    /// - For native flags, only the fragments in the original `BuildOptions` are kept.
    /// - Any native flags in this instance, for fragments that are kept, are set to the value
    ///   from this instance.
    /// - All Starlark flags from the original `BuildOptions` are kept, then all Starlark options
    ///   from this instance are added, along with the scope from their parsed `scope` attribute
    ///   when known.
    /// - Any Starlark flags which are present in both, the value from this instance is kept.
    /// - Any Starlark flags set back to their default value are removed, along with their scope.
    ///
    /// To preserve fragment trimming, this method will not expand the set of included native
    /// fragments from the original `BuildOptions`.
    ///
    /// PORT: skipped, incremental: Java memoizes the result per source (`mergeCache`).
    pub fn merge_with(&self, source: &BuildOptions) -> BuildConfigurationKey {
        self.merge_with_impl(source)
    }

    // Java: com/google/devtools/build/lib/skyframe/config/ParsedFlagsValue.java:165 ParsedFlagsValue.mergeWithImpl
    fn merge_with_impl(&self, source: &BuildOptions) -> BuildConfigurationKey {
        let mut builder = source.to_builder();

        // Handle native options.
        for option_value in self.parsing_result.all_option_values() {
            let option_definition = option_value.get_option_definition();
            // All options obtained from an options parser are guaranteed to have been defined in
            // an FragmentOptions class.
            let Some(fragment) =
                builder.get_fragment_options_by_type_id(option_definition.declaring_class_id())
            else {
                // Preserve trimming by ignoring fragments not present in the original options.
                continue;
            };
            Self::update_option_value(fragment, &option_value);
        }

        // Merge Starlark options. The scope info from the source options is already in the
        // builder, copied by source.toBuilder(). Add the scope info from this instance's parsed
        // scope attributes on top of it, then merge the values: flags reset to their default
        // value are removed by removeStarlarkOption, which also deletes the scope info just
        // added.
        builder.add_scope_type_map(BuildOptions::convert_scopes_attributes(
            self.parsing_result.get_scopes_attributes(),
            self.parsing_result.get_starlark_options(),
        ));
        for (name, value) in self.parsing_result.get_starlark_options() {
            self.update_starlark_flag(
                &mut builder,
                name,
                StarlarkOptionValue::from_option_value(value),
            );
        }

        BuildConfigurationKey::create(Arc::new(builder.build()))
    }

    // Java: com/google/devtools/build/lib/skyframe/config/ParsedFlagsValue.java:198 ParsedFlagsValue.updateOptionValue
    fn update_option_value(
        fragment: &mut dyn blaze_options::definition::OptionsBase,
        option_value: &blaze_options::value_description::OptionValueDescription,
    ) {
        // TODO: https://github.com/bazelbuild/bazel/issues/22453 - This will completely overwrite
        //  accumulating flags, which is almost certainly not what users want. Instead this should
        //  intelligently merge options.
        let value = option_value.get_value();
        let option_definition = option_value.get_option_definition();
        if option_definition.set_value(fragment, value).is_err() {
            panic!(
                "IllegalStateException: Could not set option {}",
                option_definition.get_option_name()
            );
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/config/ParsedFlagsValue.java:209 ParsedFlagsValue.updateStarlarkFlag
    fn update_starlark_flag(
        &self,
        builder: &mut BuildOptionsBuilder,
        raw_flag_name: &str,
        raw_flag_value: StarlarkOptionValue,
    ) {
        let flag_name = Label::parse_canonical_unchecked(raw_flag_name);
        // If the known default value is the same as the new value, unset it.
        if self.is_starlark_flag_set_to_default(raw_flag_name, &raw_flag_value) {
            builder.remove_starlark_option(&flag_name);
        } else {
            builder.add_starlark_option(flag_name, raw_flag_value);
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/config/ParsedFlagsValue.java:220 ParsedFlagsValue.isStarlarkFlagSetToDefault
    fn is_starlark_flag_set_to_default(
        &self,
        raw_flag_name: &str,
        raw_flag_value: &StarlarkOptionValue,
    ) -> bool {
        self.flags
            .starlark_flag_defaults
            .get(raw_flag_name)
            .is_some_and(|default_val| default_val.java_equals(raw_flag_value))
    }
}

impl PartialEq for ParsedFlagsValue {
    // Java: com/google/devtools/build/lib/skyframe/config/ParsedFlagsValue.java:226 ParsedFlagsValue.equals
    fn eq(&self, other: &Self) -> bool {
        self.flags == other.flags
    }
}

impl Eq for ParsedFlagsValue {}

impl Hash for ParsedFlagsValue {
    // Java: com/google/devtools/build/lib/skyframe/config/ParsedFlagsValue.java:237 ParsedFlagsValue.hashCode
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.flags.hash(state);
    }
}

impl fmt::Debug for ParsedFlagsValue {
    // Java: com/google/devtools/build/lib/skyframe/config/ParsedFlagsValue.java:242 ParsedFlagsValue.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ParsedFlagsValue{{flags={:?}}}", self.flags)
    }
}
