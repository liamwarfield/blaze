//! Ports of `analysis/starlark/StarlarkTransition.java` (the static validation helpers),
//! `analysis/config/StarlarkTransitionCache.java` and
//! `skyframe/StarlarkBuildSettingsDetailsValue.java` (the value and key; the function is in
//! `blaze_sky_analysis`).

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use blaze_build_config::build_options::{BuildOptions, JavaCollectionClass, StarlarkOptionValue};
use blaze_build_config::core_option_converters::BuildSettingConverter;
use blaze_cmdline::Label;
use blaze_cmdline::label_constants::COMMAND_LINE_OPTION_PACKAGE_IDENTIFIER;
use blaze_events::{Event, EventHandler, ExtendedEventHandler, StoredEventHandler};
use blaze_optdefs::fragment::core_options::CoreOptions;
use blaze_packages::types::{AttrValue, Type};
use blaze_skyframe::{Key, SkyFunctionName};
use blaze_starlark::StarlarkSemantics;
use indexmap::{IndexMap, IndexSet};
use parking_lot::Mutex;

use crate::config::transitions::{ConfigurationTransition, StarlarkTransition, restrict};

/// Exception class for exceptions thrown during application of a starlark-defined transition
/// (Java `StarlarkTransition.TransitionException`).
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkTransition.java:103 StarlarkTransition.TransitionException
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct TransitionException(pub String);

/// Java `StarlarkDefinedConfigTransition.Settings` (which settings of a transition to read).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Settings {
    Inputs,
    Outputs,
    InputsAndOutputs,
}

// -------------------------------------------------------------------------------------------
// StarlarkBuildSettingsDetailsValue.

/// `SkyFunctions.STARLARK_BUILD_SETTINGS_DETAILS`.
pub const STARLARK_BUILD_SETTINGS_DETAILS: SkyFunctionName =
    SkyFunctionName("STARLARK_BUILD_SETTINGS_DETAILS");

/// A value class for storing details about Starlark build settings read by a transition: their
/// defaults, types, whether they allow multiple values, and the aliases used to refer to them.
// Java: com/google/devtools/build/lib/skyframe/StarlarkBuildSettingsDetailsValue.java:38 StarlarkBuildSettingsDetailsValue
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StarlarkBuildSettingsDetailsValue {
    /// Java `ImmutableMap<Label, Object>`: the defaults in their `BuildOptions` form.
    pub build_setting_to_default: IndexMap<Label, StarlarkOptionValue>,
    pub build_setting_to_type: IndexMap<Label, Type>,
    pub build_setting_is_allows_multiple: IndexSet<Label>,
    pub alias_to_actual: IndexMap<Label, Label>,
}

impl StarlarkBuildSettingsDetailsValue {
    // Java: com/google/devtools/build/lib/skyframe/StarlarkBuildSettingsDetailsValue.java:52 StarlarkBuildSettingsDetailsValue.EMPTY
    pub fn empty() -> StarlarkBuildSettingsDetailsValue {
        StarlarkBuildSettingsDetailsValue::default()
    }
}

/// Java `StarlarkBuildSettingsDetailsValue.Key`.
// Java: com/google/devtools/build/lib/skyframe/StarlarkBuildSettingsDetailsValue.java:75 StarlarkBuildSettingsDetailsValue.Key
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct StarlarkBuildSettingsDetailsKey {
    /// Java `ImmutableSet<Label>` (insertion order, no duplicates).
    build_settings: Vec<Label>,
}

impl StarlarkBuildSettingsDetailsKey {
    // Java: com/google/devtools/build/lib/skyframe/StarlarkBuildSettingsDetailsValue.java:70 StarlarkBuildSettingsDetailsValue.key
    pub fn key(build_settings: &IndexSet<Label>) -> StarlarkBuildSettingsDetailsKey {
        StarlarkBuildSettingsDetailsKey {
            build_settings: build_settings.iter().cloned().collect(),
        }
    }

    pub fn build_settings(&self) -> &[Label] {
        &self.build_settings
    }
}

impl fmt::Debug for StarlarkBuildSettingsDetailsKey {
    /// Java record `toString`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Key[buildSettings=[{}]]",
            self.build_settings
                .iter()
                .map(|l| l.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

impl Key for StarlarkBuildSettingsDetailsKey {
    type Value = StarlarkBuildSettingsDetailsValue;
    const FUNCTION: SkyFunctionName = STARLARK_BUILD_SETTINGS_DETAILS;
}

/// The `BuildOptions` form (Java object) of a build setting's attribute value.
///
/// PORT: Java stores the attribute value object itself in the Starlark options map (a
/// `Boolean`, `StarlarkInt`, `String`, `Label`, an interned `ImmutableList` or a `Set`); Blaze's
/// `BuildOptions` holds [`StarlarkOptionValue`]s, so the value is converted here, keeping the
/// Java class.
pub fn attr_value_to_starlark_option(value: &AttrValue) -> StarlarkOptionValue {
    match value {
        AttrValue::Bool(b) => StarlarkOptionValue::Bool(*b),
        AttrValue::Int(i) => StarlarkOptionValue::Int(i64::from(*i)),
        AttrValue::String(s) => StarlarkOptionValue::Str(s.clone()),
        AttrValue::Label(l) => StarlarkOptionValue::Label(l.clone()),
        AttrValue::StringList(l) => {
            StarlarkOptionValue::list(l.iter().cloned().map(StarlarkOptionValue::Str).collect())
        }
        // Java: com/google/devtools/build/lib/packages/Type.java:812 Type.SetType.convert (a Set).
        AttrValue::StringSet(l) => {
            StarlarkOptionValue::set(l.iter().cloned().map(StarlarkOptionValue::Str).collect())
        }
        AttrValue::LabelList(l) => {
            StarlarkOptionValue::list(l.iter().cloned().map(StarlarkOptionValue::Label).collect())
        }
        other => blaze_util::unported!(
            "StarlarkBuildSettingsDetailsFunction.compute",
            "build setting default {:?}",
            other
        ),
    }
}

// Java: com/google/devtools/build/lib/analysis/config/CoreOptionConverters.java:76 CoreOptionConverters.BUILD_SETTING_CONVERTERS
/// `BUILD_SETTING_CONVERTERS.get(type)`: the converter that parses the string form of a build
/// setting of the given type (on the command line, in `config_setting(flag_values)`, in platform
/// `flags`). `None` where the Java map has no entry.
pub fn build_setting_converter(type_: Type) -> Option<BuildSettingConverter> {
    match type_ {
        Type::Integer => Some(BuildSettingConverter::StarlarkInt),
        Type::Boolean => Some(BuildSettingConverter::Boolean),
        Type::String => Some(BuildSettingConverter::String),
        Type::StringList => Some(BuildSettingConverter::CommaSeparatedOptionList),
        Type::StringSet => Some(BuildSettingConverter::StringSet),
        Type::Label => Some(BuildSettingConverter::Label),
        Type::LabelList => Some(BuildSettingConverter::LabelList),
        Type::NodepLabel => Some(BuildSettingConverter::Label),
        _ => None,
    }
}

// Java: com/google/devtools/build/lib/packages/Type.java:297 Type.ConversionException.message
fn conversion_exception(
    expected: &str,
    value: &StarlarkOptionValue,
    what: &str,
) -> TransitionException {
    // Java: new TransitionException(e): the message is the ConversionException's toString(),
    // which for an EvalException is its message.
    TransitionException(format!(
        "expected {} for {}, but got {} ({})",
        expected,
        what,
        value.starlark_repr(),
        starlark_type_of(value)
    ))
}

// Java: com/google/devtools/build/lib/packages/Type.java:315 Type.ConversionException.<init>
fn type_conversion_exception(
    type_: Type,
    value: &StarlarkOptionValue,
    what: &str,
) -> TransitionException {
    conversion_exception(&format!("value of type '{}'", type_), value, what)
}

/// Java `type.convert(value, what)` (no label converter) for a value read from
/// `BuildOptions.getStarlarkOptions()`. `what` is `what.toString()`: the setting's label, or
/// `element <i> of <label>` for the elements of a collection.
// Java: com/google/devtools/build/lib/packages/Type.java:84 Type.convert
fn convert_starlark_option(
    type_: Type,
    value: &StarlarkOptionValue,
    what: &str,
) -> Result<StarlarkOptionValue, TransitionException> {
    match type_ {
        // Java: com/google/devtools/build/lib/packages/Type.java:357 Type.IntegerType.convert
        Type::Integer => match value {
            StarlarkOptionValue::Int(i) => {
                // i.toIntUnchecked(): assert signed 32-bit
                if i32::try_from(*i).is_err() {
                    return Err(TransitionException(format!(
                        "for {}, got {}, want value in signed 32-bit range",
                        what, i
                    )));
                }
                Ok(value.clone())
            }
            _ => Err(type_conversion_exception(type_, value, what)),
        },
        // Java: com/google/devtools/build/lib/packages/Type.java:411 Type.BooleanType.convert
        // Conversion to boolean must also tolerate integers of 0 and 1 only.
        Type::Boolean => match value {
            StarlarkOptionValue::Bool(_) => Ok(value.clone()),
            StarlarkOptionValue::Int(0) => Ok(StarlarkOptionValue::Bool(false)),
            StarlarkOptionValue::Int(1) => Ok(StarlarkOptionValue::Bool(true)),
            _ => Err(conversion_exception(
                "one of [False, True, 0, 1]",
                value,
                what,
            )),
        },
        // Java: com/google/devtools/build/lib/packages/Type.java:469 Type.StringType.convert
        Type::String | Type::StringNoIntern => match value {
            StarlarkOptionValue::Str(_) => Ok(value.clone()),
            _ => Err(type_conversion_exception(type_, value, what)),
        },
        // Java: com/google/devtools/build/lib/packages/BuildType.java:412 BuildType.LabelType.convert
        Type::Label | Type::NodepLabel => match value {
            StarlarkOptionValue::Label(_) => Ok(value.clone()),
            StarlarkOptionValue::Str(s) => match Label::parse_canonical(s) {
                Ok(l) => Ok(StarlarkOptionValue::Label(l)),
                Err(e) => Err(TransitionException(format!(
                    "invalid label '{}' in {}: {}",
                    s,
                    what,
                    e.get_message()
                ))),
            },
            _ => Err(type_conversion_exception(Type::String, value, what)),
        },
        // Java: com/google/devtools/build/lib/packages/Type.java:730 Type.ListType.convert
        Type::StringList | Type::LabelList | Type::NodepLabelList => {
            // x instanceof Iterable (every Java collection is one); the result is an ArrayList.
            let StarlarkOptionValue::List(iterable) = value else {
                return Err(type_conversion_exception(type_, value, what));
            };
            let elem_type = type_.get_list_element_type().expect("list element type");
            let mut result = Vec::with_capacity(iterable.len());
            for (index, elem) in iterable.iter().enumerate() {
                // Java: com/google/devtools/build/lib/packages/Type.java:688 Type.CollectionType.ConversionContext.toString
                let conversion_context = format!("element {} of {}", index, what);
                result.push(convert_starlark_option(
                    elem_type,
                    elem,
                    &conversion_context,
                )?);
            }
            Ok(StarlarkOptionValue::list(result))
        }
        // Java: com/google/devtools/build/lib/packages/Type.java:812 Type.SetType.convert
        Type::StringSet => {
            let set = match value {
                StarlarkOptionValue::List(l) if l.is_java_set() => l,
                _ => return Err(type_conversion_exception(type_, value, what)),
            };
            let elem_type = type_.get_list_element_type().expect("set element type");
            let mut result = Vec::with_capacity(set.len());
            for (index, elem) in set.iter().enumerate() {
                let conversion_context = format!("element {} of {}", index, what);
                result.push(convert_starlark_option(
                    elem_type,
                    elem,
                    &conversion_context,
                )?);
            }
            // A LinkedHashSet.
            Ok(StarlarkOptionValue::set(result))
        }
        other => blaze_util::unported!(
            "StarlarkTransition.validateAndCheckIfAtDefault",
            "build setting of type {}",
            other
        ),
    }
}

/// Java `Starlark.type(value)`.
// Java: net/starlark/java/eval/Starlark.java:337 Starlark.classType
fn starlark_type_of(value: &StarlarkOptionValue) -> &'static str {
    match value {
        StarlarkOptionValue::Bool(_) => "bool",
        StarlarkOptionValue::Int(_) => "int",
        StarlarkOptionValue::Str(_) => "string",
        StarlarkOptionValue::Label(_) => "Label",
        // PORT: Starlark.classType of a FeatureFlagValue class (not a Starlark value); not
        // reached by any ported caller.
        StarlarkOptionValue::FeatureFlag(v) => {
            blaze_util::unported!("Starlark.classType", "FeatureFlagValue {}", v)
        }
        StarlarkOptionValue::List(l) => match l.class() {
            JavaCollectionClass::StarlarkList => "list",
            JavaCollectionClass::Tuple => "tuple",
            // Any class of java.util.List that isn't a Sequence.
            JavaCollectionClass::List => "List",
            // PORT: Starlark.classType prints the simple name of the concrete Set class
            // (RegularImmutableSet, LinkedHashSet, ...), which is not tracked.
            // Java: net/starlark/java/eval/StarlarkSet.java StarlarkSet (`set`).
            JavaCollectionClass::StarlarkSet => "set",
            JavaCollectionClass::Set => {
                blaze_util::unported!("Starlark.classType", "java.util.Set")
            }
        },
    }
}

// -------------------------------------------------------------------------------------------
// StarlarkTransition statics.

/// Given a transition, figures out if it composes any Starlark transitions.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkTransition.java:367 StarlarkTransition.doesStarlarkTransition
pub fn does_starlark_transition(root: &ConfigurationTransition) -> bool {
    let mut does_starlark_transition = false;
    root.visit_starlark(&mut |_| does_starlark_transition = true);
    does_starlark_transition
}

/// The command-line flag aliases of the options (`--flag_alias`), as a map.
pub fn command_line_flag_aliases(options: &BuildOptions) -> IndexMap<String, Label> {
    options
        .get::<CoreOptions>()
        .expect("CoreOptions")
        .get_command_line_flag_aliases()
        .into_iter()
        .collect()
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkTransition.java:315 StarlarkTransition.getRelevantStarlarkSettingsFromTransition
pub fn get_relevant_starlark_settings_from_transition(
    transition: &StarlarkTransition,
    flags_aliases: &IndexMap<String, Label>,
    settings: Settings,
) -> IndexSet<Label> {
    let empty = IndexMap::new();
    let flags_aliases = if transition.is_exec_transition() {
        // Ignore flag aliases for exec transitions. Starlark flags will provide their exec
        // transition semantics in the flag definition.
        &empty
    } else {
        flags_aliases
    };
    let mut result = IndexSet::new();
    match settings {
        Settings::Inputs => {
            add_label_if_relevant(&mut result, flags_aliases, &transition.get_inputs())
        }
        Settings::Outputs => {
            add_label_if_relevant(&mut result, flags_aliases, &transition.get_outputs())
        }
        Settings::InputsAndOutputs => {
            add_label_if_relevant(&mut result, flags_aliases, &transition.get_inputs());
            add_label_if_relevant(&mut result, flags_aliases, &transition.get_outputs());
        }
    }
    result
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkTransition.java:335 StarlarkTransition.addLabelIfRelevant
fn add_label_if_relevant(
    builder: &mut IndexSet<Label>,
    flags_aliases: &IndexMap<String, Label>,
    entries: &[Label],
) {
    for entry in entries {
        if *entry.get_package_identifier() != *COMMAND_LINE_OPTION_PACKAGE_IDENTIFIER {
            builder.insert(entry.clone());
        } else {
            let flag_name = entry.get_name();
            if let Some(alias_target) = flags_aliases.get(flag_name) {
                builder.insert(alias_target.clone());
            }
        }
    }
}

/// Method to be called after Starlark-transitions are applied. Checks outputs.
///
/// We only do validation on Starlark-defined build settings. Native options already have their
/// output values checked in `FunctionTransitionUtil.applyTransition`.
///
/// Remove build settings in `to_options` that have been set to their default value. This is how
/// we ensure that an unset build setting and a set-to-default build settings represent the same
/// configuration.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkTransition.java:150 StarlarkTransition.validate
pub fn validate(
    root: &ConfigurationTransition,
    details: &StarlarkBuildSettingsDetailsValue,
    flags_aliases: &IndexMap<String, Label>,
    to_options: IndexMap<String, BuildOptions>,
) -> Result<IndexMap<String, BuildOptions>, TransitionException> {
    // Collect settings that are inputs or outputs of the transition together with their types.
    // Output setting values will be validated and removed if set to their default.
    // Raw means these have not been unaliased.
    let mut raw_input_and_output_settings: IndexSet<Label> = IndexSet::new();
    // Collect settings that were only used as inputs to the transition and thus possibly had
    // their default values added to the fromOptions. They will be removed if set to ther
    // default, but should not be validated.
    let mut input_only_settings: IndexSet<Label> = IndexSet::new();
    root.visit_starlark(&mut |transition| {
        let input_and_output_settings = get_relevant_starlark_settings_from_transition(
            transition,
            flags_aliases,
            Settings::InputsAndOutputs,
        );
        let output_settings = get_relevant_starlark_settings_from_transition(
            transition,
            flags_aliases,
            Settings::Outputs,
        );
        for setting in input_and_output_settings {
            if !output_settings.contains(&setting) {
                input_only_settings.insert(setting.clone());
            }
            raw_input_and_output_settings.insert(setting);
        }
    });

    // Return early if the transition has neither inputs nor outputs (rare).
    if raw_input_and_output_settings.is_empty() {
        return Ok(to_options);
    }

    // Verify changed settings were changed to something reasonable for their type and filter
    // out default values.
    let mut cleaned_option_map: IndexMap<String, BuildOptions> = IndexMap::new();
    for (key, value) in to_options {
        // Lazily initialized to optimize for the common case where we don't modify anything.
        let mut cleaned_options = None;
        // Clean up aliased values.
        let options = unalias(value, &details.alias_to_actual);
        for maybe_alias_setting in &raw_input_and_output_settings {
            // Note that if the build setting may be referenced in the transition via an alias
            let setting = details
                .alias_to_actual
                .get(maybe_alias_setting)
                .unwrap_or(maybe_alias_setting);
            // Input-only settings may have had their literal default value added to the
            // BuildOptions so that the transition can read them. We have to remove these
            // explicitly set value here to preserve the invariant that Starlark settings at
            // default values are not explicitly set in the BuildOptions.
            // Java: details.buildSettingToDefault().get(setting).equals(
            //     options.getStarlarkOptions().get(setting))
            let is_input_only_setting_at_default =
                input_only_settings.contains(maybe_alias_setting) && {
                    let default = details
                        .build_setting_to_default
                        .get(setting)
                        .unwrap_or_else(|| {
                            panic!("NullPointerException: no default of {}", setting)
                        });
                    match options.get_starlark_options().get(setting) {
                        Some(value) => default.java_equals(value),
                        // equals(null)
                        None => false,
                    }
                };
            // For output settings, the raw value returned by the transition first has to be
            // validated and converted to the proper type before it can be compared to the
            // default value.
            if is_input_only_setting_at_default
                || validate_and_check_if_at_default(
                    details,
                    &options,
                    maybe_alias_setting,
                    setting,
                    &raw_input_and_output_settings,
                )?
            {
                cleaned_options
                    .get_or_insert_with(|| options.to_builder())
                    .remove_starlark_option(setting);
            }
        }
        // Keep the same instance if we didn't do anything to maintain reference equality later
        // on.
        let options = match cleaned_options {
            Some(mut b) => b.build(),
            None => options,
        };
        if cleaned_option_map.contains_key(&key) {
            panic!("IllegalArgumentException: Multiple entries with same key: {key}");
        }
        cleaned_option_map.insert(key, options);
    }
    Ok(cleaned_option_map)
}

/// Validate the value of a particular build setting after a transition has been applied.
/// Returns true if and only if the setting is set to its default value after the transition.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkTransition.java:238 StarlarkTransition.validateAndCheckIfAtDefault
fn validate_and_check_if_at_default(
    details: &StarlarkBuildSettingsDetailsValue,
    options: &BuildOptions,
    maybe_alias_setting: &Label,
    setting: &Label,
    input_and_output_settings: &IndexSet<Label>,
) -> Result<bool, TransitionException> {
    let new_value = options.get_starlark_options().get(setting);
    // TODO(b/154132845): fix NPE occasionally observed here.
    let Some(new_value) = new_value else {
        panic!(
            "IllegalStateException: Error while attempting to validate new values from starlark transition(s) with the inputs and outputs [{}]. Post-transition configuration should include '{}' but only includes starlark options: [{}]. If you run into this error please ping b/154132845 or email blaze-configurability@google.com.",
            input_and_output_settings
                .iter()
                .map(|l| l.to_string())
                .collect::<Vec<_>>()
                .join(", "),
            setting,
            options
                .get_starlark_options()
                .keys()
                .map(|l| l.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
    };
    let allows_multiple = details.build_setting_is_allows_multiple.contains(setting);
    let type_ = *details
        .build_setting_to_type
        .get(setting)
        .expect("type of a build setting");
    let default = details.build_setting_to_default.get(setting);
    let what = maybe_alias_setting.to_string();
    if allows_multiple {
        // if this setting allows multiple settings
        // Java: !(newValue instanceof List<?> rawNewValueAsList)
        let raw_new_value_as_list = match new_value {
            StarlarkOptionValue::List(l) if l.is_java_list() => l,
            _ => {
                return Err(TransitionException(format!(
                    "'{}' allows multiple values and must be set in transition using a starlark list instead of single value '{}'",
                    setting,
                    new_value.java_to_string()
                )));
            }
        };
        let mut converted_value = Vec::new();
        for value in raw_new_value_as_list.iter() {
            converted_value.push(convert_starlark_option(type_, value, &what)?);
        }
        // Java: convertedValue.equals(ImmutableList.of(details.buildSettingToDefault()
        //     .get(setting))).
        // The details' default of an allow-multiple setting is already
        // `ImmutableList.of(rawDefault)` (StarlarkBuildSettingsDetailsFunction.java:93), so the
        // right-hand side is a one-element list whose element is that list: the comparison is
        // only true if the converted value is a list containing exactly that list, which the
        // element type (a string) never allows. A setting that allows multiple values therefore
        // stays in the configuration even when a transition sets it to its default.
        let converted_value = StarlarkOptionValue::list(converted_value);
        let default = default.unwrap_or_else(|| {
            panic!(
                "NullPointerException: ImmutableList.of(null): no default of {}",
                setting
            )
        });
        Ok(converted_value.java_equals(&StarlarkOptionValue::list(vec![default.clone()])))
    } else {
        // if this setting does not allow multiple settings
        let converted_value = convert_starlark_option(type_, new_value, &what)?;
        // Java: convertedValue.equals(details.buildSettingToDefault().get(setting))
        Ok(default.is_some_and(|default| converted_value.java_equals(default)))
    }
}

/// Resolve aliased build setting issues: collapse `alias -> new value` and `actual -> old value`
/// entries to `actual -> new value`.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkTransition.java:285 StarlarkTransition.unalias
fn unalias(options: BuildOptions, alias_to_actual: &IndexMap<Label, Label>) -> BuildOptions {
    if alias_to_actual.is_empty() {
        return options;
    }
    let mut to_return = options.to_builder();
    for (setting, value) in options.get_starlark_options() {
        if alias_to_actual.values().any(|a| a == setting) {
            // if entry is keyed by an actual, don't care about its value it's stale
            continue;
        }
        if let Some(actual) = alias_to_actual.get(setting) {
            // if an entry is keyed by an alias, newly key (overwrite) its actual to its alias'
            // value and remove the alias-keyed entry
            to_return.add_starlark_option(actual.clone(), value.clone());
            to_return.remove_starlark_option(setting);
        } else {
            // else - just copy over
            to_return.add_starlark_option(setting.clone(), value.clone());
        }
    }
    to_return.build()
}

// -------------------------------------------------------------------------------------------
// StarlarkTransitionCache.

/// Caches the results of Starlark transition application (Java `StarlarkTransitionCache`,
/// owned by `SkyframeBuildView`).
///
/// PORT: one instance per build arena ([`StarlarkTransitionCache::get`]; `blaze_util::build_arena`,
/// decision D7): Java's instance belongs to the `SkyframeBuildView` and is cleared at every
/// `clearLegacyData` (each command that resets the evaluator, as the first Blaze server does),
/// at `clearAnalysisCache` and at `clearInvalidatedActionLookupKeys`; the server releases the
/// arena after every command. Java's soft-valued Caffeine caches become plain maps.
// Java: com/google/devtools/build/lib/analysis/config/StarlarkTransitionCache.java:43 StarlarkTransitionCache
#[derive(Default)]
pub struct StarlarkTransitionCache {
    cache: Mutex<HashMap<CacheKey, CacheValue>>,
    /// Cache of the set of Starlark build settings referenced by a transition.
    starlark_build_settings_cache: Mutex<HashMap<String, IndexSet<Label>>>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    transition: String,
    from_options: String,
    details: String,
}

#[derive(Clone)]
struct CacheValue {
    result: IndexMap<String, BuildOptions>,
    /// Non-error events (`print()` output) of a successful transition.
    non_error_events: Option<Vec<Event>>,
}

impl blaze_util::build_arena::ArenaSlot for StarlarkTransitionCache {
    // Its keys name transition definitions by the address of an `Arc` the arena's
    // `ConvertedTransitions` memo keeps.
    const RELEASE_ORDER: blaze_util::build_arena::ReleaseOrder =
        blaze_util::build_arena::ReleaseOrder::State;
}

impl StarlarkTransitionCache {
    /// The build arena's cache (Java `SkyframeBuildView.getStarlarkTransitionCache()`).
    pub fn get() -> Arc<StarlarkTransitionCache> {
        blaze_util::build_arena::slot::<StarlarkTransitionCache>()
    }

    /// Given a transition, decompose (if possible) and find all referenced Starlark build
    /// settings, exactly as referenced (aliases are not resolved). Flag aliases used in the
    /// transition contribute the Starlark flag they map to.
    // Java: com/google/devtools/build/lib/analysis/config/StarlarkTransitionCache.java:72 StarlarkTransitionCache.getAllStarlarkBuildSettings
    pub fn get_all_starlark_build_settings(
        &self,
        root: &ConfigurationTransition,
        flags_aliases: &IndexMap<String, Label>,
    ) -> IndexSet<Label> {
        let cache_key = root.cache_key();
        if let Some(cached_value) = self.starlark_build_settings_cache.lock().get(&cache_key) {
            return cached_value.clone();
        }

        let mut key_builder: IndexSet<Label> = IndexSet::new();
        root.visit_starlark(&mut |transition| {
            key_builder.extend(get_relevant_starlark_settings_from_transition(
                transition,
                flags_aliases,
                Settings::InputsAndOutputs,
            ));
        });

        self.starlark_build_settings_cache
            .lock()
            .insert(cache_key, key_builder.clone());
        key_builder
    }

    /// Adds the default values for a transition's input build settings to its input build
    /// options.
    // Java: com/google/devtools/build/lib/analysis/config/StarlarkTransitionCache.java:98 StarlarkTransitionCache.addDefaultStarlarkOptions
    fn add_default_starlark_options(
        &self,
        from_options: &BuildOptions,
        flags_aliases: &IndexMap<String, Label>,
        transition: &ConfigurationTransition,
        details: &StarlarkBuildSettingsDetailsValue,
    ) -> BuildOptions {
        if details.build_setting_to_default.is_empty() {
            // No need to traverse the transition to find its Starlark flag inputs. There are
            // none.
            return from_options.clone();
        }

        let mut options_with_defaults = None;
        for maybe_alias_setting in self.get_all_starlark_build_settings(transition, flags_aliases) {
            // details will only have the defaults of the actual setting so must unalias
            let setting = details
                .alias_to_actual
                .get(&maybe_alias_setting)
                .unwrap_or(&maybe_alias_setting)
                .clone();
            if !from_options
                .get_starlark_options()
                .contains_key(&maybe_alias_setting)
            {
                let default = details
                    .build_setting_to_default
                    .get(&setting)
                    .expect("default of a build setting")
                    .clone();
                options_with_defaults
                    .get_or_insert_with(|| from_options.to_builder())
                    .add_starlark_option(maybe_alias_setting.clone(), default);
            }
        }
        match options_with_defaults {
            None => from_options.clone(),
            Some(mut b) => b
                .add_scope_type_map(from_options.get_scope_type_map().clone())
                .build(),
        }
    }

    /// Applies a Starlark transition, possibly returning a cached result.
    // Java: com/google/devtools/build/lib/analysis/config/StarlarkTransitionCache.java:134 StarlarkTransitionCache.computeIfAbsent
    pub fn compute_if_absent(
        &self,
        from_options: &BuildOptions,
        transition: &ConfigurationTransition,
        details: &StarlarkBuildSettingsDetailsValue,
        event_handler: &dyn ExtendedEventHandler,
        semantics: &Arc<StarlarkSemantics>,
    ) -> Result<IndexMap<String, BuildOptions>, TransitionException> {
        let cache_key = CacheKey {
            transition: transition.cache_key(),
            from_options: from_options.checksum().to_owned(),
            details: format!("{details:?}"),
        };
        let cached_result = self.cache.lock().get(&cache_key).cloned();
        if let Some(cached_result) = cached_result {
            if let Some(events) = &cached_result.non_error_events {
                Event::replay_events_on(event_handler, events);
            }
            return Ok(cached_result.result);
        }

        let flags_aliases = command_line_flag_aliases(from_options);

        // All code below here only executes on a cache miss and thus should rely only on values
        // that are part of the above cache key or constants that exist throughout the lifetime
        // of the Blaze server instance.
        let adjusted_options =
            self.add_default_starlark_options(from_options, &flags_aliases, transition, details);
        let handler_with_error_status = StoredEventHandler::new();
        let result = transition.apply(
            &restrict(transition, &adjusted_options),
            &handler_with_error_status,
            semantics,
        );

        // We use a temporary StoredEventHandler instead of the caller's event handler because
        // StarlarkTransition.validate assumes no errors occurred. We need a StoredEventHandler to
        // be able to check that, and fail out early if there are errors.
        handler_with_error_status.replay_on(event_handler);
        if handler_with_error_status.has_errors() {
            return Err(TransitionException(
                "Errors encountered while applying Starlark transition".to_owned(),
            ));
        }
        let result = validate(transition, details, &flags_aliases, result)?;
        // If the transition errored (like bad Starlark code), this method already exited with an
        // exception so the results won't go into the cache. We still want to collect non-error
        // events like print() output.
        let non_error_events = if !handler_with_error_status.is_empty() {
            Some(handler_with_error_status.get_events())
        } else {
            None
        };
        self.cache.lock().insert(
            cache_key,
            CacheValue {
                result: result.clone(),
                non_error_events,
            },
        );
        Ok(result)
    }

    // Java: com/google/devtools/build/lib/analysis/config/StarlarkTransitionCache.java:185 StarlarkTransitionCache.clear
    pub fn clear(&self) {
        self.cache.lock().clear();
        self.starlark_build_settings_cache.lock().clear();
    }
}

/// `EventHandler` for `Event`s only, used where Java hands a handler that never receives posts.
pub fn handle_all(handler: &dyn EventHandler, events: &[Event]) {
    for e in events {
        handler.handle(e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn details_key_to_string() {
        let mut s = IndexSet::new();
        s.insert(Label::parse_canonical_unchecked("//a:b"));
        s.insert(Label::parse_canonical_unchecked("//c:d"));
        assert_eq!(
            format!("{:?}", StarlarkBuildSettingsDetailsKey::key(&s)),
            "Key[buildSettings=[//a:b, //c:d]]"
        );
    }

    #[test]
    fn attr_value_defaults_convert() {
        assert_eq!(
            attr_value_to_starlark_option(&AttrValue::String("3.11".into())),
            StarlarkOptionValue::Str("3.11".into())
        );
        assert_eq!(
            attr_value_to_starlark_option(&AttrValue::Bool(true)),
            StarlarkOptionValue::Bool(true)
        );
    }

    #[test]
    #[ignore = "needs a Starlark transition fixture (StarlarkRuleTransitionProviderTest)"]
    fn test_aliased_build_setting_output_return_mismatch() {}

    fn s(v: &str) -> StarlarkOptionValue {
        StarlarkOptionValue::Str(v.to_string())
    }

    // Expected texts: the oracle's transcript of scripts/w4/scenarios/flags-transitions
    // (steps 15 to 21): Type.ConversionException prints the value with repr and names the
    // element of a list.
    #[test]
    fn conversion_error_texts() {
        let err = |type_: Type, value: StarlarkOptionValue, what: &str| {
            convert_starlark_option(type_, &value, what).unwrap_err().0
        };
        assert_eq!(
            err(Type::Integer, s("abc"), "//:level"),
            "expected value of type 'int' for //:level, but got \"abc\" (string)"
        );
        assert_eq!(
            err(Type::Integer, StarlarkOptionValue::Bool(true), "//:level"),
            "expected value of type 'int' for //:level, but got True (bool)"
        );
        assert_eq!(
            err(
                Type::StringList,
                StarlarkOptionValue::starlark_list(vec![s("a"), StarlarkOptionValue::Int(1)]),
                "//:copts"
            ),
            "expected value of type 'string' for element 1 of //:copts, but got 1 (int)"
        );
        assert_eq!(
            err(Type::StringList, s("a"), "//:copts"),
            "expected value of type 'list(string)' for //:copts, but got \"a\" (string)"
        );
        assert_eq!(
            err(Type::Boolean, s("yes"), "//:enabled"),
            "expected one of [False, True, 0, 1] for //:enabled, but got \"yes\" (string)"
        );
        assert_eq!(
            err(Type::String, StarlarkOptionValue::Int(5), "//:mode"),
            "expected value of type 'string' for //:mode, but got 5 (int)"
        );
    }

    // Expected: the oracle accepts `0` for a bool setting (flags-transitions step 20), and the
    // converted list of a string_list setting is a plain java.util.List.
    #[test]
    fn conversions() {
        assert_eq!(
            convert_starlark_option(Type::Boolean, &StarlarkOptionValue::Int(0), "//:enabled"),
            Ok(StarlarkOptionValue::Bool(false))
        );
        assert_eq!(
            convert_starlark_option(
                Type::StringList,
                &StarlarkOptionValue::starlark_list(vec![s("c0")]),
                "//:copts"
            ),
            Ok(StarlarkOptionValue::list(vec![s("c0")]))
        );
    }

    fn details_of(
        setting: &Label,
        default: StarlarkOptionValue,
        type_: Type,
        allows_multiple: bool,
    ) -> StarlarkBuildSettingsDetailsValue {
        let mut details = StarlarkBuildSettingsDetailsValue::empty();
        details
            .build_setting_to_default
            .insert(setting.clone(), default);
        details.build_setting_to_type.insert(setting.clone(), type_);
        if allows_multiple {
            details
                .build_setting_is_allows_multiple
                .insert(setting.clone());
        }
        details
    }

    fn at_default(
        details: &StarlarkBuildSettingsDetailsValue,
        setting: &Label,
        value: StarlarkOptionValue,
    ) -> Result<bool, TransitionException> {
        let options = BuildOptions::builder()
            .add_starlark_option(setting.clone(), value)
            .build();
        let settings: IndexSet<Label> = [setting.clone()].into_iter().collect();
        validate_and_check_if_at_default(details, &options, setting, setting, &settings)
    }

    // Expected: flags-transitions step 7 (the oracle keeps `//:multi` in the configuration when
    // a transition sets the allow_multiple setting to `["m0"]`, its default) and step 11.
    #[test]
    fn allow_multiple_setting_set_to_its_default_is_not_at_default() {
        let setting = Label::parse_canonical_unchecked("//:multi");
        // StarlarkBuildSettingsDetailsFunction: ImmutableList.of(rawDefault).
        let details = details_of(
            &setting,
            StarlarkOptionValue::list(vec![s("m0")]),
            Type::String,
            true,
        );
        assert_eq!(
            at_default(
                &details,
                &setting,
                StarlarkOptionValue::starlark_list(vec![s("m0")])
            ),
            Ok(false)
        );
        assert_eq!(
            at_default(&details, &setting, s("a")),
            Err(TransitionException(
                "'//:multi' allows multiple values and must be set in transition using a starlark list instead of single value 'a'"
                    .to_string()
            ))
        );
    }

    // Expected: flags-transitions steps 4 and 14 (a transition that sets a setting to its default
    // gives the configuration without the setting).
    #[test]
    fn setting_set_to_its_default_is_at_default() {
        let copts = Label::parse_canonical_unchecked("//:copts");
        let details = details_of(
            &copts,
            StarlarkOptionValue::list(vec![s("c0")]),
            Type::StringList,
            false,
        );
        assert_eq!(
            at_default(
                &details,
                &copts,
                StarlarkOptionValue::starlark_list(vec![s("c0")])
            ),
            Ok(true)
        );
        assert_eq!(
            at_default(
                &details,
                &copts,
                StarlarkOptionValue::starlark_list(vec![s("-DX")])
            ),
            Ok(false)
        );
        let level = Label::parse_canonical_unchecked("//:level");
        let details = details_of(&level, StarlarkOptionValue::Int(1), Type::Integer, false);
        assert_eq!(
            at_default(&details, &level, StarlarkOptionValue::Int(1)),
            Ok(true)
        );
    }
}
