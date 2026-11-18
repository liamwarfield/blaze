//! Port of `runtime/StarlarkOptionsParser.java`: the options parser for Starlark-defined options,
//! and of `BlazeOptionHandler.SkyframeExecutorTargetLoader` (`runtime/BlazeOptionHandler.java`),
//! the loader used for top-level flag parsing.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use blaze_analysis::config::starlark_transition::{
    attr_value_to_starlark_option, build_setting_converter,
};
use blaze_build_config::build_options::{JavaCollectionClass, ScopeType, StarlarkOptionValue};
use blaze_cmdline::{Label, TargetParsingException, TargetPatternsCode};
use blaze_events::{Event, EventHandler, ExtendedEventHandler, Postable};
use blaze_options::{OptionsParser, OptionsParsingException, Value};
use blaze_packages::attribute::{AttributeMap, RawAttributeMapper};
use blaze_packages::rule_class::build_setting::BuildSetting;
use blaze_packages::rule_class::rule_class::STARLARK_BUILD_SETTING_DEFAULT_ATTR_NAME;
use blaze_packages::target::{RuleOrMacroInstance, Target};
use blaze_packages::types::{AttrValue, Type};
use blaze_sky_loading::package_manager::NoSuchPackageOrInterrupted;
use blaze_sky_top::skyframe_executor::{
    DEFAULT_THREAD_COUNT, SkyframeExecutor, TargetPatternPhaseError,
};
use blaze_vfs::PathFragment;
use indexmap::{IndexMap, IndexSet};

/// What `BuildSettingLoader.loadBuildSetting` throws (`InterruptedException`,
/// `TargetParsingException`).
#[derive(Clone, Debug)]
pub enum BuildSettingLoadError {
    Interrupted,
    TargetParsing(TargetParsingException),
}

/// Interface for caller-specific logic to convert flag names to `Target`s.
///
/// The most important distinction is whether the caller is in a `SkyFunction` evaluation
/// environment.
// Java: com/google/devtools/build/lib/runtime/StarlarkOptionsParser.java:71 StarlarkOptionsParser.BuildSettingLoader
pub trait BuildSettingLoader {
    /// Converts a flag name into a `Target`, or returns an error if this can't be done.
    ///
    /// `name` is the flag to lookup, expected to be a valid `Label`. Returns the `Target`
    /// corresponding to the flag, or `None` if the caller has to do more work to retrieve the
    /// target (after which it'll call this parser again).
    // Java: com/google/devtools/build/lib/runtime/StarlarkOptionsParser.java:80 StarlarkOptionsParser.BuildSettingLoader.loadBuildSetting
    fn load_build_setting(&self, name: &str) -> Result<Option<Target>, BuildSettingLoadError>;
}

/// What `StarlarkOptionsParser.parse` throws (`InterruptedException`,
/// `OptionsParsingException`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StarlarkOptionsParsingError {
    Interrupted,
    OptionsParsing(OptionsParsingException),
}

impl From<OptionsParsingException> for StarlarkOptionsParsingError {
    fn from(e: OptionsParsingException) -> Self {
        StarlarkOptionsParsingError::OptionsParsing(e)
    }
}

/// A helper class to create new instances of [`StarlarkOptionsParser`].
// Java: com/google/devtools/build/lib/runtime/StarlarkOptionsParser.java:90 StarlarkOptionsParser.Builder
pub struct StarlarkOptionsParserBuilder<'a> {
    build_setting_loader: Option<&'a dyn BuildSettingLoader>,
    native_options_parser: Option<&'a mut OptionsParser>,
    include_default_values: bool,
}

impl<'a> StarlarkOptionsParserBuilder<'a> {
    /// Set the `BuildSettingLoader` used to find flags.
    pub fn build_setting_loader(
        mut self,
        build_setting_loader: &'a dyn BuildSettingLoader,
    ) -> Self {
        self.build_setting_loader = Some(build_setting_loader);
        self
    }

    /// Sets the native `OptionsParser` used for handling flags.
    pub fn native_options_parser(mut self, native_options_parser: &'a mut OptionsParser) -> Self {
        self.native_options_parser = Some(native_options_parser);
        self
    }

    /// Whether or not to report Starlark flags which are set to their default values.
    pub fn include_default_values(mut self, include_default_values: bool) -> Self {
        self.include_default_values = include_default_values;
        self
    }

    /// Returns a new [`StarlarkOptionsParser`].
    pub fn build(self) -> StarlarkOptionsParser<'a> {
        StarlarkOptionsParser {
            native_options_parser: self
                .native_options_parser
                .expect("IllegalStateException: Missing required properties: nativeOptionsParser"),
            build_setting_loader: self
                .build_setting_loader
                .expect("IllegalStateException: Missing required properties: buildSettingLoader"),
            starlark_options: BTreeMap::new(),
            scopes: BTreeMap::new(),
            parsed_build_settings: IndexMap::new(),
            build_settings: HashMap::new(),
            build_setting_defaults: IndexMap::new(),
            include_default_values: self.include_default_values,
        }
    }
}

/// An options parser for starlark defined options. Takes a mutable `OptionsParser` that has
/// already parsed all native options (including those needed for loading). This class is in
/// charge of parsing and setting the starlark options for this `OptionsParser`.
// Java: com/google/devtools/build/lib/runtime/StarlarkOptionsParser.java:62 StarlarkOptionsParser
pub struct StarlarkOptionsParser<'a> {
    native_options_parser: &'a mut OptionsParser,

    build_setting_loader: &'a dyn BuildSettingLoader,

    // TODO: https://github.com/bazelbuild/bazel/issues/22365 - Unify these maps into a common
    // data structure. Consider using OptionDefinition to simplify.

    // Result of #parse, store the parsed options and their values.
    // PORT: a TreeMap<String, Object> (Java String order; Rust orders by UTF-8 bytes, which
    // differs only for labels with characters outside the Basic Multilingual Plane).
    starlark_options: BTreeMap<String, StarlarkOptionValue>,

    // Map of starlark options to their `Scope.ScopeType`.
    scopes: BTreeMap<String, String>,

    // Map of parsed starlark options to their loaded BuildSetting objects (used for
    // canonicalization)
    parsed_build_settings: IndexMap<String, BuildSetting>,

    // Local cache of build settings so we don't repeatedly load them.
    build_settings: HashMap<String, Target>,

    // The default value for each build setting.
    build_setting_defaults: IndexMap<String, StarlarkOptionValue>,

    // whether options explicitly set to their default values are added to `starlark_options`
    include_default_values: bool,
}

/// An unparsed option: the option name (canonical label), its unparsed value and the loaded
/// option (Java `Map.Entry<String, Pair<String, Target>>` of the `LinkedListMultimap`).
type UnparsedOption = (String, (String, Target));

fn build_setting_of(build_setting_target: &Target) -> &BuildSetting {
    // buildSettingTarget.getAssociatedRule().getRuleClassObject().getBuildSetting()
    build_setting_target
        .get_associated_rule()
        .expect("NullPointerException: getAssociatedRule()")
        .get_rule_class_object()
        .get_build_setting()
        .expect("NullPointerException: getBuildSetting()")
}

impl<'a> StarlarkOptionsParser<'a> {
    /// Create a new [`StarlarkOptionsParserBuilder`] instance for `StarlarkOptionsParser`.
    // Java: com/google/devtools/build/lib/runtime/StarlarkOptionsParser.java:84 StarlarkOptionsParser.builder
    pub fn builder() -> StarlarkOptionsParserBuilder<'a> {
        StarlarkOptionsParserBuilder {
            build_setting_loader: None,
            native_options_parser: None,
            include_default_values: false,
        }
    }

    /// Parses all pre "--" residue for Starlark options.
    ///
    /// Returns true if the flags are parsed, false if the `BuildSettingLoader` needs to do more
    /// work to retrieve build setting targets (after which it'll call this method again).
    // TODO(blaze-configurability): This method somewhat reinvents the wheel of
    // OptionsParserImpl.identifyOptionAndPossibleArgument. Consider combining. This would
    // probably require multiple rounds of parsing to fit starlark-defined options into native
    // option format.
    // Java: com/google/devtools/build/lib/runtime/StarlarkOptionsParser.java:148 StarlarkOptionsParser.parse
    pub fn parse(&mut self) -> Result<bool, StarlarkOptionsParsingError> {
        let skipped_args = self.native_options_parser.get_skipped_args();
        self.parse_given_args(&skipped_args)
    }

    /// Parses a specific set of flags.
    ///
    /// Returns true if the flags are parsed, false if the `BuildSettingLoader` needs to do more
    /// work to retrieve build setting targets (after which it'll call this method again).
    // Java: com/google/devtools/build/lib/runtime/StarlarkOptionsParser.java:159 StarlarkOptionsParser.parseGivenArgs
    pub fn parse_given_args(
        &mut self,
        args: &[String],
    ) -> Result<bool, StarlarkOptionsParsingError> {
        // Map of <option name (label), <unparsed option value, loaded option>>.
        let mut unparsed_options: Vec<UnparsedOption> = Vec::new();

        let mut all_targets_available = true;
        for arg in args {
            if !self.parse_arg(arg, &mut unparsed_options)? {
                all_targets_available = false;
            }
        }

        if !all_targets_available {
            return Ok(false);
        } else if unparsed_options.is_empty() {
            return Ok(true);
        }

        // Map of flag label as a string to its loaded target and set value after parsing.
        // PORT: java-hash-order. Java's HashMap is iterated below (keySet()); the entries are
        // kept in insertion order here and visited in Java's order
        // (`java_hash_map_key_order`).
        let mut build_setting_with_target_and_value: IndexMap<
            String,
            (Target, StarlarkOptionValue),
        > = IndexMap::new();
        for (loaded_flag, (unparsed_value, build_setting_target)) in &unparsed_options {
            let build_setting = build_setting_of(build_setting_target);
            // Do not recognize internal options, which are treated as if they did not exist.
            if !build_setting.is_flag() {
                return Err(OptionsParsingException::new(format!(
                    "Unrecognized option: {}={}",
                    loaded_flag, unparsed_value
                ))
                .into());
            }
            let mut type_ = build_setting.get_type();
            if build_setting.is_repeatable_flag() {
                type_ = type_
                    .get_list_element_type()
                    .expect("NullPointerException: type.getListElementType()");
            }
            let converter = build_setting_converter(type_).unwrap_or_else(|| {
                panic!(
                    "NullPointerException: BUILD_SETTING_CONVERTERS has no converter for type {}",
                    type_
                )
            });
            let mut value = match converter.convert(
                unparsed_value,
                self.native_options_parser
                    .get_conversion_context()
                    .map(|c| &**c),
            ) {
                Ok(value) => value,
                Err(e) => {
                    return Err(OptionsParsingException::with_cause(
                        format!(
                            "While parsing option {}={}: '{}' is not a {}",
                            loaded_flag, unparsed_value, unparsed_value, type_
                        ),
                        e.get_message(),
                    )
                    .into());
                }
            };
            if build_setting.allows_multiple() || build_setting.is_repeatable_flag() {
                // (Collection<?>) buildSettingWithTargetAndValue.get(loadedFlag).getSecond()
                let mut new_value: Vec<StarlarkOptionValue> =
                    match build_setting_with_target_and_value.get(loaded_flag) {
                        Some((_, StarlarkOptionValue::List(previous))) => previous.items().to_vec(),
                        Some((_, other)) => panic!(
                            "ClassCastException: {} cannot be cast to java.util.Collection",
                            other.java_to_string()
                        ),
                        None => Vec::new(),
                    };
                new_value.push(value);
                value = if build_setting.get_type() == Type::StringSet {
                    // A LinkedHashSet.
                    StarlarkOptionValue::set(new_value)
                } else {
                    // An ArrayList.
                    StarlarkOptionValue::list(new_value)
                };
            }
            build_setting_with_target_and_value
                .insert(loaded_flag.clone(), (build_setting_target.clone(), value));
        }

        let mut parsed_options: BTreeMap<String, StarlarkOptionValue> = BTreeMap::new();
        let mut scope_type_map: BTreeMap<String, String> = BTreeMap::new();
        let keys: Vec<&String> = build_setting_with_target_and_value.keys().collect();
        for index in java_hash_map_key_order(&keys) {
            let build_setting = keys[index];
            let (build_setting_target, value) = &build_setting_with_target_and_value[index];
            let rule = build_setting_target
                .get_associated_rule()
                .expect("NullPointerException: getAssociatedRule()");
            let build_setting_object = build_setting_of(build_setting_target);
            let allows_multiple = build_setting_object.allows_multiple();
            self.parsed_build_settings
                .insert(build_setting.clone(), build_setting_object.clone());
            let raw_default_value: Option<StarlarkOptionValue> = rule
                .get_attr(STARLARK_BUILD_SETTING_DEFAULT_ATTR_NAME)
                .map(|default| attr_value_to_starlark_option(&default));
            if allows_multiple {
                // ImmutableList.of(Objects.requireNonNull(rawDefaultValue))
                let default_value = StarlarkOptionValue::list(vec![
                    raw_default_value.expect("NullPointerException: rawDefaultValue"),
                ]);
                self.build_setting_defaults
                    .insert(build_setting.clone(), default_value.clone());
                // List<?> newValue = (List<?>) value;
                if matches!(value, StarlarkOptionValue::List(l) if l.class() == JavaCollectionClass::Set)
                    || !matches!(value, StarlarkOptionValue::List(_))
                {
                    panic!(
                        "ClassCastException: {} cannot be cast to java.util.List",
                        value.java_to_string()
                    );
                }
                if !value.java_equals(&default_value) || self.include_default_values {
                    parsed_options.insert(build_setting.clone(), value.clone());
                }
            } else {
                if let Some(raw_default_value) = &raw_default_value {
                    self.build_setting_defaults
                        .insert(build_setting.clone(), raw_default_value.clone());
                }
                // !value.equals(rawDefaultValue)
                let equals_default = raw_default_value
                    .as_ref()
                    .is_some_and(|raw_default_value| value.java_equals(raw_default_value));
                if !equals_default || self.include_default_values {
                    parsed_options.insert(build_setting.clone(), value.clone());
                }
            }

            // TODO: b/384058698 - use NonConfigurableAttributeMapper to ensure "scope" isn't
            // selectable.
            let attr_map = RawAttributeMapper::of(&**rule);
            let mut scope_type = ScopeType::DEFAULT.name().to_string();
            if attr_map.is_attribute_value_explicitly_specified("scope") {
                scope_type = match attr_map.get("scope", Type::String) {
                    Some(AttrValue::String(scope)) => scope,
                    other => panic!(
                        "ClassCastException: scope attribute of {} is {:?}",
                        build_setting, other
                    ),
                };
                if !ScopeType::allowed_attribute_values().contains(&scope_type.to_lowercase()) {
                    return Err(OptionsParsingException::new(format!(
                        "Can't load flag --{}: Invalid \"scope\" attribute value \"{}\". Allowed values: [{}].",
                        build_setting,
                        scope_type,
                        ScopeType::allowed_attribute_values()
                            .iter()
                            .map(|s| format!("\"{}\"", s))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ))
                    .into());
                }
            }
            scope_type_map.insert(build_setting.clone(), scope_type);
            self.native_options_parser
                .set_scopes_attributes(scope_type_map.clone());
        }

        self.native_options_parser.set_starlark_options(
            parsed_options
                .iter()
                .map(|(name, value)| (name.clone(), Value::new(value.clone())))
                .collect(),
        );
        self.starlark_options.extend(parsed_options);
        self.scopes.extend(scope_type_map);
        Ok(true)
    }

    /// Parses the given `flag=value` setting.
    ///
    /// Returns true if parsing finishes, false if the `BuildSettingLoader` needs to do more work
    /// to retrieve the build setting target.
    // Java: com/google/devtools/build/lib/runtime/StarlarkOptionsParser.java:289 StarlarkOptionsParser.parseArg
    fn parse_arg(
        &mut self,
        arg: &str,
        unparsed_options: &mut Vec<UnparsedOption>,
    ) -> Result<bool, StarlarkOptionsParsingError> {
        if !arg.starts_with("--") {
            return Err(OptionsParsingException::with_argument(
                format!("Invalid options syntax: {}", arg),
                arg,
            )
            .into());
        }
        // This isn't resilient against labels with the "=" character in them, e.g.
        // "//pkg/prefix=suffix". See https://bazel.build/concepts/labels#target-names.
        let equals_at = arg.find('=');
        let mut name = match equals_at {
            None => &arg[2..],
            Some(equals_at) => &arg[2..equals_at],
        };
        // Java String.trim(): strips characters up to U+0020.
        if name.trim_matches(|c: char| c <= ' ').is_empty() {
            return Err(OptionsParsingException::with_argument(
                format!("Invalid options syntax: {}", arg),
                arg,
            )
            .into());
        }
        let value = equals_at.map(|equals_at| &arg[equals_at + 1..]);

        if let Some(value) = value {
            // --flag=value or -flag=value form
            let Some(build_setting_target) = self.load_build_setting(name)? else {
                return Ok(false);
            };
            // Use the canonical form to ensure we don't have
            // duplicate options getting into the starlark options map.
            unparsed_options.push((
                build_setting_target.get_label().get_canonical_form(),
                (value.to_string(), build_setting_target),
            ));
        } else {
            let mut boolean_value = true;
            // check --noflag form
            if let Some(rest) = name.strip_prefix("no") {
                boolean_value = false;
                name = rest;
            }
            let Some(build_setting_target) = self.load_build_setting(name)? else {
                return Ok(false);
            };
            let current = build_setting_of(&build_setting_target);
            if current.get_type() == Type::Boolean {
                // --boolean_flag or --noboolean_flag
                // Ditto w/r/t canonical form.
                unparsed_options.push((
                    build_setting_target.get_label().get_canonical_form(),
                    (boolean_value.to_string(), build_setting_target),
                ));
            } else {
                if !boolean_value {
                    // --no(non_boolean_flag)
                    return Err(OptionsParsingException::with_argument(
                        format!("Illegal use of 'no' prefix on non-boolean option: {}", name),
                        name,
                    )
                    .into());
                }
                return Err(OptionsParsingException::with_argument(
                    format!("Expected value after {}", arg),
                    arg,
                )
                .into());
            }
        }
        Ok(true)
    }

    /// Returns the given build setting's `Target`, following (unconfigured) aliases if needed.
    ///
    /// Returns the target, or `None` if the `BuildSettingLoader` needs to do more work to
    /// retrieve the target.
    // Java: com/google/devtools/build/lib/runtime/StarlarkOptionsParser.java:351 StarlarkOptionsParser.loadBuildSetting
    fn load_build_setting(
        &mut self,
        target_to_build: &str,
    ) -> Result<Option<Target>, StarlarkOptionsParsingError> {
        if let Some(target) = self.build_settings.get(target_to_build) {
            return Ok(Some(target.clone()));
        }

        let mut target: Target;
        let mut target_to_load_next = target_to_build.to_string();
        // SequencedSet<Label> aliasChain = new LinkedHashSet<>();
        let mut alias_chain: IndexSet<Label> = IndexSet::new();
        loop {
            target = match self
                .build_setting_loader
                .load_build_setting(&target_to_load_next)
            {
                Ok(Some(target)) => target,
                Ok(None) => return Ok(None),
                Err(BuildSettingLoadError::Interrupted) => {
                    return Err(StarlarkOptionsParsingError::Interrupted);
                }
                Err(BuildSettingLoadError::TargetParsing(e)) => {
                    return Err(OptionsParsingException::with_argument_and_cause(
                        format!(
                            "Error loading option {}: {}",
                            target_to_build,
                            e.get_message()
                        ),
                        target_to_build,
                        e.get_message(),
                    )
                    .into());
                }
            };
            if !alias_chain.insert(target.get_label().clone()) {
                return Err(OptionsParsingException::with_argument(
                    format!(
                        "Failed to load build setting '{}' due to a cycle in alias chain: {}",
                        target_to_build,
                        format_alias_chain(
                            alias_chain
                                .iter()
                                .chain(std::iter::once(target.get_label()))
                        )
                    ),
                    target_to_build,
                )
                .into());
            }
            let Some(associated_rule) = target.get_associated_rule() else {
                return Err(OptionsParsingException::with_argument(
                    format!(
                        "Unrecognized option: {}",
                        format_alias_chain(alias_chain.iter())
                    ),
                    target_to_build,
                )
                .into());
            };
            if associated_rule.is_build_setting() {
                break;
            }
            // Follow the unconfigured values of aliases.
            if associated_rule.get_rule_class() == "alias" {
                target_to_load_next = match associated_rule.get_attr("actual") {
                    Some(AttrValue::Label(label)) => label.get_unambiguous_canonical_form(),
                    Some(AttrValue::SelectorList(_)) => {
                        return Err(OptionsParsingException::with_argument(
                            format!(
                                "Failed to load build setting '{}' as it resolves to an alias with an actual value that uses select(): {}. This is not supported as build settings are needed to determine the configuration the select is evaluated in.",
                                target_to_build,
                                format_alias_chain(alias_chain.iter())
                            ),
                            target_to_build,
                        )
                        .into());
                    }
                    _ => panic!(
                        "IllegalStateException: Alias target '{}' with 'actual' attr value not equals to a label or a selectorlist",
                        target.get_label()
                    ),
                };
                continue;
            }
            return Err(OptionsParsingException::with_argument(
                format!(
                    "Unrecognized option: {}",
                    format_alias_chain(alias_chain.iter())
                ),
                target_to_build,
            )
            .into());
        }

        self.build_settings
            .insert(target_to_build.to_string(), target.clone());
        Ok(Some(target))
    }

    // Java: com/google/devtools/build/lib/runtime/StarlarkOptionsParser.java:423 StarlarkOptionsParser.getStarlarkOptions
    pub fn get_starlark_options(&self) -> &BTreeMap<String, StarlarkOptionValue> {
        &self.starlark_options
    }

    // Java: com/google/devtools/build/lib/runtime/StarlarkOptionsParser.java:427 StarlarkOptionsParser.getScopesAttributes
    pub fn get_scopes_attributes(&self) -> &BTreeMap<String, String> {
        &self.scopes
    }

    // Java: com/google/devtools/build/lib/runtime/StarlarkOptionsParser.java:431 StarlarkOptionsParser.getDefaultValues
    pub fn get_default_values(&self) -> &IndexMap<String, StarlarkOptionValue> {
        &self.build_setting_defaults
    }

    // Java: com/google/devtools/build/lib/runtime/StarlarkOptionsParser.java:435 StarlarkOptionsParser.checkIfParsedOptionAllowsMultiple
    pub fn check_if_parsed_option_allows_multiple(&self, option: &str) -> bool {
        let setting = self
            .parsed_build_settings
            .get(option)
            .expect("NullPointerException: parsedBuildSettings.get(option)");
        setting.allows_multiple() || setting.is_repeatable_flag()
    }

    // Java: com/google/devtools/build/lib/runtime/StarlarkOptionsParser.java:440 StarlarkOptionsParser.getParsedOptionType
    pub fn get_parsed_option_type(&self, option: &str) -> Type {
        self.parsed_build_settings
            .get(option)
            .expect("NullPointerException: parsedBuildSettings.get(option)")
            .get_type()
    }

    // Java: com/google/devtools/build/lib/runtime/StarlarkOptionsParser.java:445 StarlarkOptionsParser.getDefaultValue
    pub fn get_default_value(&self, option: &str) -> Option<&StarlarkOptionValue> {
        self.build_setting_defaults.get(option)
    }

    /// Return a canoncalized list of the starlark options and values that this parser has
    /// parsed.
    // Java: com/google/devtools/build/lib/runtime/StarlarkOptionsParser.java:451 StarlarkOptionsParser.canonicalize
    pub fn canonicalize(&self) -> Vec<String> {
        let mut result: Vec<String> = Vec::new();
        for (starlark_option_name, starlark_option_value) in &self.starlark_options {
            let starlark_option_string = format!("--{}=", starlark_option_name);
            if self.check_if_parsed_option_allows_multiple(starlark_option_name) {
                let StarlarkOptionValue::List(collection) = starlark_option_value else {
                    panic!(
                        "IllegalStateException: Found a starlark option value that isn't a list or set for an allow multiple option."
                    );
                };
                for single_value in collection.iter() {
                    result.push(format!(
                        "{}{}",
                        starlark_option_string,
                        single_value.java_to_string()
                    ));
                }
            } else if self.get_parsed_option_type(starlark_option_name) == Type::StringList
                || self.get_parsed_option_type(starlark_option_name) == Type::StringSet
            {
                // String.join(",", (Iterable<String>) starlarkOptionValue)
                let StarlarkOptionValue::List(strings) = starlark_option_value else {
                    panic!(
                        "ClassCastException: {} cannot be cast to java.lang.Iterable",
                        starlark_option_value.java_to_string()
                    );
                };
                result.push(format!(
                    "{}{}",
                    starlark_option_string,
                    strings
                        .iter()
                        .map(StarlarkOptionValue::java_to_string)
                        .collect::<Vec<_>>()
                        .join(",")
                ));
            } else {
                result.push(format!(
                    "{}{}",
                    starlark_option_string,
                    starlark_option_value.java_to_string()
                ));
            }
        }
        result
    }
}

// Java: com/google/devtools/build/lib/runtime/StarlarkOptionsParser.java:419 StarlarkOptionsParser.formatAliasChain
fn format_alias_chain<'l>(alias_chain: impl Iterator<Item = &'l Label>) -> String {
    alias_chain
        .map(Label::get_canonical_form)
        .collect::<Vec<_>>()
        .join(" -> ")
}

/// The order in which Java iterates `HashMap<String, V>.keySet()` after the keys were `put` in
/// the given order: the indices of `keys`.
///
/// `java.util.HashMap` spreads `String.hashCode()` (`h ^ (h >>> 16)`), starts with 16 buckets and
/// doubles the table whenever the size exceeds 0.75 times the bucket count; iteration goes
/// through the buckets in index order, and through one bucket in insertion order (a resize keeps
/// the relative order of the entries of a bucket).
/// PORT: buckets with eight or more colliding entries become trees in Java, which can reorder
/// them; that needs at least eight flag labels with the same spread hash bits.
// Java: java/util/HashMap.java HashMap.hash, HashMap.resize, HashMap.HashIterator (JDK)
pub fn java_hash_map_key_order(keys: &[&String]) -> Vec<usize> {
    let mut capacity: usize = 16;
    while keys.len() > capacity / 4 * 3 {
        capacity *= 2;
    }
    let bucket = |key: &str| -> usize {
        let h =
            blaze_build_config::output_path_mnemonic_computer::java_string_hash_code(key) as u32;
        ((h ^ (h >> 16)) as usize) & (capacity - 1)
    };
    let mut order: Vec<usize> = (0..keys.len()).collect();
    // Stable: entries of one bucket stay in insertion order.
    order.sort_by_key(|&index| bucket(keys[index]));
    order
}

/// Lets [`StarlarkOptionsParser`] convert flag names to `Target`s through
/// `TargetPatternPhaseValue`.
///
/// This is used for top-level flag parsing, outside any `SkyFunction`.
// Java: com/google/devtools/build/lib/runtime/BlazeOptionHandler.java:371 BlazeOptionHandler.SkyframeExecutorTargetLoader
pub struct SkyframeExecutorTargetLoader<'a> {
    skyframe_executor: &'a SkyframeExecutor,
    relative_working_directory: PathFragment,
    reporter: Arc<dyn ExtendedEventHandler>,
}

impl<'a> SkyframeExecutorTargetLoader<'a> {
    // Java: com/google/devtools/build/lib/runtime/BlazeOptionHandler.java:384 BlazeOptionHandler.SkyframeExecutorTargetLoader.<init>
    pub fn new(
        skyframe_executor: &'a SkyframeExecutor,
        relative_working_directory: PathFragment,
        reporter: Arc<dyn ExtendedEventHandler>,
    ) -> Self {
        SkyframeExecutorTargetLoader {
            skyframe_executor,
            relative_working_directory,
            reporter: Arc::new(NonPostingEventHandler { delegate: reporter }),
        }
    }
}

impl BuildSettingLoader for SkyframeExecutorTargetLoader<'_> {
    // Java: com/google/devtools/build/lib/runtime/BlazeOptionHandler.java:394 BlazeOptionHandler.SkyframeExecutorTargetLoader.loadBuildSetting
    fn load_build_setting(
        &self,
        target_label: &str,
    ) -> Result<Option<Target>, BuildSettingLoadError> {
        let tpv = match self.skyframe_executor.load_target_patterns_without_filters(
            self.reporter.clone(),
            &[target_label.to_string()],
            &self.relative_working_directory,
            DEFAULT_THREAD_COUNT,
            /* keep_going= */ false,
        ) {
            Ok(tpv) => tpv,
            Err(TargetPatternPhaseError::Interrupted) => {
                return Err(BuildSettingLoadError::Interrupted);
            }
            Err(TargetPatternPhaseError::TargetParsing(e)) => {
                return Err(BuildSettingLoadError::TargetParsing(e));
            }
        };
        // tpv.getTargets(reporter, skyframeExecutor.getPackageManager())
        // Java: com/google/devtools/build/lib/skyframe/TargetPatternPhaseValue.java:66 TargetPatternPhaseValue.getTargetsFromLabels
        let mut result: Vec<Target> = Vec::new();
        for label in tpv.get_target_labels() {
            let package = match self
                .skyframe_executor
                .get_package_manager()
                .get_package(self.reporter.clone(), label.get_package_identifier())
            {
                Ok(package) => package,
                Err(NoSuchPackageOrInterrupted::Interrupted) => {
                    return Err(BuildSettingLoadError::Interrupted);
                }
                Err(NoSuchPackageOrInterrupted::NoSuchPackage(e)) => panic!(
                    "IllegalStateException: Failed to get preloaded package from TargetPatternPhaseValue for {}: {}",
                    label, e
                ),
            };
            match package.get_target(label.get_name()) {
                Ok(target) => result.push(target.clone()),
                Err(e) => panic!(
                    "IllegalStateException: Failed to get preloaded package from TargetPatternPhaseValue for {}: {}",
                    label,
                    e.get_message()
                ),
            }
        }
        if result.len() != 1 {
            return Err(BuildSettingLoadError::TargetParsing(
                TargetParsingException::new(
                    "user-defined flags must reference exactly one target",
                    TargetPatternsCode::TargetFormatInvalid,
                ),
            ));
        }
        Ok(result.pop())
    }
}

/// `ExtendedEventHandler` override that passes through "normal" events but not events that would
/// go to the build event proto.
///
/// Starlark flags are conceptually options but still need target pattern evaluation in
/// `BlazeOptionHandler.parseStarlarkOptions` to translate their labels to actual targets. If we
/// pass the `post`able events from that evaluation, that would produce "target loaded" and
/// "target configured" events in the build event proto output that consumers can confuse with
/// actual targets requested by the build.
///
/// Hence this class. By dropping those events, we restrict all info and error reporting logic to
/// the options parsing pipeline.
// Java: com/google/devtools/build/lib/runtime/BlazeOptionHandler.java:343 BlazeOptionHandler.NonPostingEventHandler
struct NonPostingEventHandler {
    delegate: Arc<dyn ExtendedEventHandler>,
}

impl EventHandler for NonPostingEventHandler {
    fn handle(&self, e: &Event) {
        self.delegate.handle(e);
    }

    fn as_extended_event_handler(&self) -> Option<&dyn ExtendedEventHandler> {
        Some(self)
    }
}

impl ExtendedEventHandler for NonPostingEventHandler {
    fn post(&self, _e: Arc<dyn Postable>) {
        // Fetches of external repositories are not reported as BES events and important to
        // surface in the CLI due to their long-running nature.
        // PORT: Java passes `FetchProgress` posts on to the delegate. Whether a `dyn Postable`
        // is a `FetchProgress` cannot be asked in `blaze_events` (no `as_fetch_progress`), so
        // those progress posts are dropped too; they only feed the progress display.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Expected values: java.util.HashMap iteration order (JDK): "Aa" and "BB" share
    // String.hashCode() 2112, so they share a bucket and keep their insertion order; the
    // single-character keys "a".."d" have hash codes 97..100 and land in buckets 1..4 of 16;
    // 2112 is bucket 0.
    #[test]
    fn java_hash_map_key_order_follows_buckets() {
        let keys: Vec<String> = ["d", "BB", "b", "Aa", "a", "c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let refs: Vec<&String> = keys.iter().collect();
        let order: Vec<&str> = java_hash_map_key_order(&refs)
            .into_iter()
            .map(|i| keys[i].as_str())
            .collect();
        assert_eq!(order, vec!["BB", "Aa", "a", "b", "c", "d"]);
    }
}
