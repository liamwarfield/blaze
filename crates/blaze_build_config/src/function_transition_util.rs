//! Port of `analysis/starlark/FunctionTransitionUtil.java`, the application half of
//! `StarlarkAttributeTransitionProvider.FunctionSplitTransition`, and
//! `analysis/config/ExecutionTransitionFactory.java` (the exec transition finalizer).

use std::collections::BTreeSet;
use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_cmdline::label_constants::{
    COMMAND_LINE_OPTION_PACKAGE_IDENTIFIER, COMMAND_LINE_OPTION_PREFIX,
};
use blaze_events::{Event, EventHandler};
use blaze_optdefs::fragment::converters::CustomFlagConverter;
use blaze_optdefs::fragment::core_options::{
    CoreOptions, CoreOptionsExecConfigurationDistinguisherScheme,
    CoreOptionsOutputDirectoryNamingScheme,
};
use blaze_optdefs::fragment::platform_options::PlatformOptions;
use blaze_options::{Foreign, OptionMetadataTag, OptionsParsingException, Value as OptionValue};
use blaze_starlark::StarlarkSemantics;
use indexmap::{IndexMap, IndexSet};

use crate::build_options::{
    BuildOptions, BuildOptionsView, JavaCollectionClass, ScopeType, StarlarkOptionValue,
};
use crate::fragment_options_class::{CORE_OPTIONS, PLATFORM_OPTIONS, TEST_OPTIONS};
use crate::options_diff::{OptionInfo, OptionsDiff};
pub use crate::starlark_transition::TransitionAttrObject;
use crate::starlark_transition::{
    JavaObject, PATCH_TRANSITION_KEY, StarlarkDefinedConfigTransition, ValidationException,
};

// Java: com/google/devtools/build/lib/analysis/starlark/FunctionTransitionUtil.java:100 FunctionTransitionUtil.applyAndValidate
/// Figure out what build settings the given transition changes and apply those changes to the
/// incoming `BuildOptions`. For native options, this involves a preprocess step of converting
/// options to their "command line form". Returns `None` if errors were reported to handler.
pub fn apply_and_validate(
    from_options: &BuildOptions,
    starlark_transition: &StarlarkDefinedConfigTransition,
    allow_non_configurable_flag_changes: bool,
    is_exec_transition: bool,
    semantics: &Arc<StarlarkSemantics>,
    handler: &dyn EventHandler,
) -> Option<IndexMap<String, BuildOptions>> {
    apply_and_validate_with_attrs(
        from_options,
        starlark_transition,
        allow_non_configurable_flag_changes,
        is_exec_transition,
        &crate::starlark_transition::EmptyTransitionAttrs,
        semantics,
        handler,
    )
}

/// [`apply_and_validate`] with the `attr` object the transition implementation receives (Java:
/// the transition's `attrObject`, the target's attribute values).
// W4-analysis: rule and attribute transitions read `attr`; the exec transition gets an empty one.
pub fn apply_and_validate_with_attrs(
    from_options: &BuildOptions,
    starlark_transition: &StarlarkDefinedConfigTransition,
    allow_non_configurable_flag_changes: bool,
    is_exec_transition: bool,
    attr: &dyn TransitionAttrObject,
    semantics: &Arc<StarlarkSemantics>,
    handler: &dyn EventHandler,
) -> Option<IndexMap<String, BuildOptions>> {
    let result = (|| -> Result<Option<IndexMap<String, BuildOptions>>, ValidationException> {
        let option_info_map = OptionInfo::build_map_from(from_options);
        let flags_aliases: IndexMap<String, Label> = if is_exec_transition {
            // Ignore flag aliases for exec transitions. Starlark flags will provide their exec
            // transition semantics in the flag definition.
            IndexMap::new()
        } else {
            from_options
                .get::<CoreOptions>()
                .expect("CoreOptions")
                .get_command_line_flag_aliases()
                .into_iter()
                .collect()
        };

        validate_input_options(
            &starlark_transition.get_inputs(),
            allow_non_configurable_flag_changes,
            &option_info_map,
            &flags_aliases,
        )?;
        validate_output_options(
            &starlark_transition.get_outputs(),
            allow_non_configurable_flag_changes,
            &option_info_map,
            &flags_aliases,
        )?;

        let settings = build_settings(
            from_options,
            &option_info_map,
            &flags_aliases,
            starlark_transition,
        )?;

        let mut split_build_options: IndexMap<String, BuildOptions> = IndexMap::new();

        // For anything except the exec transition this is just fromOptions. See
        // maybeGetExecDefaults for why the exec transition is different.
        let exec_defaults;
        let baseline_to_options = match maybe_get_exec_defaults(from_options, starlark_transition) {
            Some(o) => {
                exec_defaults = o;
                &exec_defaults
            }
            None => from_options,
        };

        let Some(transitions) = starlark_transition.evaluate_with_attrs(
            &settings,
            &option_info_map,
            attr,
            semantics,
            handler,
        ) else {
            return Ok(None); // errors reported to handler
        };
        if transitions.is_empty() {
            // The transition produced a no-op.
            let mut m = IndexMap::new();
            m.insert(
                PATCH_TRANSITION_KEY.to_string(),
                baseline_to_options.clone(),
            );
            return Ok(Some(m));
        }

        for (key, values) in transitions {
            let new_values = handle_implicit_platform_change(
                baseline_to_options,
                apply_starlark_flags_aliases(&flags_aliases, values)?,
            );
            let transitioned_options = apply_transition(
                baseline_to_options,
                &new_values,
                &option_info_map,
                starlark_transition,
            )?;
            split_build_options.insert(key, transitioned_options);
        }
        Ok(Some(split_build_options))
    })();
    match result {
        Ok(r) => r,
        Err(ex) => {
            let prefix = match starlark_transition.get_location() {
                Some(l) => format!("{}: ", l),
                None => String::new(),
            };
            handler.handle(&Event::error(format!("{}{}", prefix, ex.0)));
            None
        }
    }
}

// Java: com/google/devtools/build/lib/analysis/starlark/FunctionTransitionUtil.java:139 FunctionTransitionUtil.maybeGetExecDefaults
/// For all transitions except the exec transition, returns `None` (use `fromOptions`).
///
/// The exec transition is special: any options not explicitly set by the transition take their
/// defaults, not `fromOptions`'s values.
fn maybe_get_exec_defaults(
    from_options: &BuildOptions,
    starlark_transition: &StarlarkDefinedConfigTransition,
) -> Option<BuildOptions> {
    if !starlark_transition.is_exec_transition() {
        // Not an exec transition: the baseline options are just the input options.
        return None;
    }
    let mut default_builder = BuildOptions::builder();
    // Get the defaults:
    for (class, _) in from_options.get_native_options() {
        default_builder.add_fragment_options_dyn(class, class.get_default().as_ref());
    }
    // Propagate Starlark options from the source config if allowed.
    default_builder.add_starlark_options(get_exec_propagating_starlark_flags(
        from_options.get_starlark_options(),
        from_options,
    ));
    // Hard-code TestConfiguration for now, which clones the source options.
    if let Some(test_options) = from_options.get_dyn(&TEST_OPTIONS) {
        default_builder.remove_fragment_options(&TEST_OPTIONS);
        default_builder.add_fragment_options_dyn(&TEST_OPTIONS, test_options);
    }
    let mut ans = default_builder.build();
    let from_core = from_options.get::<CoreOptions>().expect("CoreOptions");
    let variables = if from_core.exclude_defines_from_exec_config {
        from_core
            .command_line_build_variables
            .iter()
            .filter(|define| from_core.custom_flags_to_propagate.contains(&define.0))
            .cloned()
            .collect()
    } else {
        from_core.command_line_build_variables.clone()
    };
    ans.get_mut::<CoreOptions>()
        .expect("CoreOptions")
        .command_line_build_variables = variables;
    Some(ans)
}

// Java: com/google/devtools/build/lib/analysis/starlark/FunctionTransitionUtil.java:198 FunctionTransitionUtil.getExecPropagatingStarlarkFlags
/// Filters a map of Starlark flag <Label, value> pairs to those that should propagate from the
/// target configuration to exec configuration.
fn get_exec_propagating_starlark_flags(
    starlark_options: &IndexMap<Label, StarlarkOptionValue>,
    options: &BuildOptions,
) -> IndexMap<Label, StarlarkOptionValue> {
    for flag in starlark_options.keys() {
        if !options.get_scope_type_map().contains_key(flag) {
            panic!(
                "VerifyException: No scope info available for Starlark flag {}.",
                flag
            );
        }
    }
    let core = options.get::<CoreOptions>().expect("CoreOptions");
    let scope_of = |l: &Label| options.get_scope_type_map().get(l).copied();
    if !core.exclude_starlark_flags_from_exec_config {
        // Starlark flags propagate to exec by default. This can only be changed by a flag
        // explicitly setting "scope = 'target'".
        return starlark_options
            .iter()
            .filter(|(k, _)| {
                matches!(
                    scope_of(k),
                    Some(ScopeType::UNIVERSAL | ScopeType::PROJECT | ScopeType::DEFAULT)
                )
            })
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
    }

    // --incompatible_exclude_starlark_flags_from_exec_config=True: Starlark flags don't
    // propagate to the exec config by default.
    let (custom_propagating_flag_patterns, custom_propagating_flags): (Vec<&String>, Vec<&String>) =
        core.custom_flags_to_propagate
            .iter()
            .partition(|f| f.ends_with(CustomFlagConverter::SUBPACKAGES_SUFFIX));
    let mut ans = IndexMap::new();
    for (key, value) in starlark_options {
        let scope = scope_of(key);
        let canonical = key.get_unambiguous_canonical_form();
        if matches!(scope, Some(ScopeType::UNIVERSAL | ScopeType::PROJECT)) {
            ans.insert(key.clone(), value.clone());
        } else if scope == Some(ScopeType::TARGET) {
            // Don't propagate this flag.
        } else if custom_propagating_flags.iter().any(|f| **f == canonical) {
            ans.insert(key.clone(), value.clone());
        } else if custom_propagating_flag_patterns.iter().any(|pattern| {
            let idx = pattern
                .rfind(CustomFlagConverter::SUBPACKAGES_SUFFIX)
                .expect("suffix");
            canonical.starts_with(&pattern[..idx])
        }) {
            ans.insert(key.clone(), value.clone());
        }
    }
    ans
}

fn command_line_option(name: &str) -> Label {
    Label::create_unvalidated(COMMAND_LINE_OPTION_PACKAGE_IDENTIFIER.clone(), name)
}

// Java: com/google/devtools/build/lib/analysis/starlark/FunctionTransitionUtil.java:152 FunctionTransitionUtil.handleImplicitPlatformChange
/// If the transition changes --cpu but not --platforms, clear out --platforms.
fn handle_implicit_platform_change(
    options: &BuildOptions,
    raw_transition_output: IndexMap<Label, JavaObject>,
) -> IndexMap<Label, JavaObject> {
    let cpu_option = command_line_option("cpu");
    let platforms_option = command_line_option("platforms");
    let new_cpu = raw_transition_output.get(&cpu_option);
    let current_cpu = &options.get::<CoreOptions>().expect("CoreOptions").cpu;
    match new_cpu {
        None => return raw_transition_output,
        Some(JavaObject::Str(s)) if s == current_cpu => return raw_transition_output,
        Some(_) => {}
    }
    if raw_transition_output.contains_key(&platforms_option) {
        // Explicitly setting --platforms overrides the implicit clearing.
        return raw_transition_output;
    }
    let mut out = raw_transition_output;
    // ImmutableMap.Builder.buildOrThrow: the key is new.
    out.insert(platforms_option, JavaObject::List(Vec::new()));
    out
}

const TRUE_STRINGS: [&str; 2] = ["true", "1"];
const FALSE_STRINGS: [&str; 2] = ["false", "0"];

// Java: com/google/devtools/build/lib/analysis/starlark/FunctionTransitionUtil.java:153 FunctionTransitionUtil.applyStarlarkFlagsAliases
/// Set the Starlark flag value to the value of its alias.
fn apply_starlark_flags_aliases(
    flags_aliases: &IndexMap<String, Label>,
    raw_transition_output: IndexMap<Label, JavaObject>,
) -> Result<IndexMap<Label, JavaObject>, ValidationException> {
    if flags_aliases.is_empty() {
        return Ok(raw_transition_output);
    }
    let mut result = raw_transition_output.clone();
    for (alias, starlark_flag) in flags_aliases {
        let native_flag = command_line_option(alias);
        let starlark_value = raw_transition_output.get(starlark_flag);
        let native_value = raw_transition_output.get(&native_flag);
        if let (Some(starlark_value), Some(native_value)) = (starlark_value, native_value) {
            let mut mismatch = false;
            if let JavaObject::Bool(bool_value) = starlark_value {
                // Supports migrating Tristate native flags to boolean Starlark flags. The former
                // appear as strings. But if those strings are "false", "true", etc. those are
                // valid booleans.
                let lower = native_value.java_to_string().to_lowercase();
                if *bool_value && !TRUE_STRINGS.contains(&lower.as_str()) {
                    mismatch = true;
                } else if !*bool_value && !FALSE_STRINGS.contains(&lower.as_str()) {
                    mismatch = true;
                }
            } else if starlark_value != native_value {
                mismatch = true;
            }
            if mismatch {
                return Err(ValidationException(format!(
                    "Starlark flag '{}' and its alias '{}' have different values: '{}' and '{}'",
                    starlark_flag,
                    native_flag,
                    starlark_value.java_to_string(),
                    native_value.java_to_string()
                )));
            }
        }
        if let Some(native_value) = native_value {
            // Add the starlark flag to the result, using the value of the alias.
            let v = if matches!(starlark_value, Some(JavaObject::Bool(_))) {
                // Boolean.parseBoolean
                JavaObject::Bool(native_value.java_to_string().eq_ignore_ascii_case("true"))
            } else {
                native_value.clone()
            };
            result.insert(starlark_flag.clone(), v);
            // Remove the entry of the alias.
            result.shift_remove(&native_flag);
        }
    }
    Ok(result)
}

// Java: com/google/devtools/build/lib/analysis/starlark/FunctionTransitionUtil.java:391 FunctionTransitionUtil.isNativeOptionValid
fn is_native_option_valid(
    option_info_map: &IndexMap<String, OptionInfo>,
    flags_aliases: &IndexMap<String, Label>,
    option_name: &str,
) -> bool {
    // Make sure the option exists, or it is an alias.
    option_info_map.contains_key(option_name) || flags_aliases.contains_key(option_name)
}

// Java: com/google/devtools/build/lib/analysis/starlark/FunctionTransitionUtil.java:405 FunctionTransitionUtil.isNativeOptionNonConfigurable
fn is_native_option_non_configurable(
    option_info_map: &IndexMap<String, OptionInfo>,
    flags_aliases: &IndexMap<String, Label>,
    option_name: &str,
) -> bool {
    match option_info_map.get(option_name) {
        None => {
            if flags_aliases.contains_key(option_name) {
                // All aliases are configurable (for now).
                return false;
            }
            panic!(
                "VerifyException: Cannot check if option {} is non-configurable: it does not exist",
                option_name
            );
        }
        Some(info) => info.has_option_metadata_tag(OptionMetadataTag::NON_CONFIGURABLE),
    }
}

// Java: com/google/devtools/build/lib/analysis/starlark/FunctionTransitionUtil.java:121 FunctionTransitionUtil.validateInputOptions
fn validate_input_options(
    options: &[String],
    allow_non_configurable_flag_changes: bool,
    option_info_map: &IndexMap<String, OptionInfo>,
    flags_aliases: &IndexMap<String, Label>,
) -> Result<(), ValidationException> {
    check_for_invalid_native_options("inputs", options, option_info_map, flags_aliases)?;
    check_for_non_configurable_options(
        "inputs",
        options,
        allow_non_configurable_flag_changes,
        option_info_map,
        flags_aliases,
    )
}

// Java: com/google/devtools/build/lib/analysis/starlark/FunctionTransitionUtil.java:126 FunctionTransitionUtil.validateOutputOptions
fn validate_output_options(
    options: &[String],
    allow_non_configurable_flag_changes: bool,
    option_info_map: &IndexMap<String, OptionInfo>,
    flags_aliases: &IndexMap<String, Label>,
) -> Result<(), ValidationException> {
    if options.iter().any(|o| o == "//command_line_option:define") {
        return Err(ValidationException(
            "Starlark transition on --define not supported - try using build settings (https://bazel.build/rules/config#user-defined-build-settings)."
                .to_string(),
        ));
    }
    check_for_invalid_native_options("outputs", options, option_info_map, flags_aliases)?;
    check_for_non_configurable_options(
        "outputs",
        options,
        allow_non_configurable_flag_changes,
        option_info_map,
        flags_aliases,
    )
}

// Java: com/google/devtools/build/lib/analysis/starlark/FunctionTransitionUtil.java:427 FunctionTransitionUtil.checkForInvalidNativeOptions
fn check_for_invalid_native_options(
    transition_parameter_type: &str,
    options: &[String],
    option_info_map: &IndexMap<String, OptionInfo>,
    flags_aliases: &IndexMap<String, Label>,
) -> Result<(), ValidationException> {
    let invalid_native_options: Vec<&str> = options
        .iter()
        .filter(|o| o.starts_with(COMMAND_LINE_OPTION_PREFIX))
        .filter(|o| {
            !is_native_option_valid(
                option_info_map,
                flags_aliases,
                &o[COMMAND_LINE_OPTION_PREFIX.len()..],
            )
        })
        .map(|s| s.as_str())
        .collect();
    if !invalid_native_options.is_empty() {
        return Err(ValidationException(format!(
            "transition {} [{}] do not correspond to valid settings",
            transition_parameter_type,
            invalid_native_options.join(", ")
        )));
    }
    Ok(())
}

// Java: com/google/devtools/build/lib/analysis/starlark/FunctionTransitionUtil.java:430 FunctionTransitionUtil.checkForNonConfigurableOptions
fn check_for_non_configurable_options(
    transition_parameter_type: &str,
    options: &[String],
    allow_non_configurable_flag_changes: bool,
    option_info_map: &IndexMap<String, OptionInfo>,
    flags_aliases: &IndexMap<String, Label>,
) -> Result<(), ValidationException> {
    if !allow_non_configurable_flag_changes {
        let non_configurable_native_options: Vec<&str> = options
            .iter()
            .filter(|o| o.starts_with(COMMAND_LINE_OPTION_PREFIX))
            .filter(|o| {
                is_native_option_non_configurable(
                    option_info_map,
                    flags_aliases,
                    &o[COMMAND_LINE_OPTION_PREFIX.len()..],
                )
            })
            .map(|s| s.as_str())
            .collect();
        if !non_configurable_native_options.is_empty() {
            return Err(ValidationException(format!(
                "transition {} [{}] cannot be changed: they are non-configurable",
                transition_parameter_type,
                non_configurable_native_options.join(", ")
            )));
        }
    }
    Ok(())
}

// Java: com/google/devtools/build/lib/analysis/starlark/FunctionTransitionUtil.java:133 FunctionTransitionUtil.buildSettings
/// Return a map containing only BuildOptions explicitly registered as transition inputs.
/// nulls are converted to Starlark.NONE but no other conversions are done.
fn build_settings(
    build_options: &BuildOptions,
    option_info_map: &IndexMap<String, OptionInfo>,
    flags_aliases: &IndexMap<String, Label>,
    starlark_transition: &StarlarkDefinedConfigTransition,
) -> Result<IndexMap<String, JavaObject>, ValidationException> {
    let inputs_canonicalized_to_given = starlark_transition.get_inputs_canonicalized_to_given();
    let mut options_builder: IndexMap<String, JavaObject> = IndexMap::new();

    // Convert the canonical form to the user requested form that they expect to see.
    for (canonical, given) in inputs_canonicalized_to_given {
        if canonical.get_package_identifier() == &*COMMAND_LINE_OPTION_PACKAGE_IDENTIFIER {
            if let Some(option_value) =
                find_native_option_value(build_options, option_info_map, flags_aliases, canonical)
            {
                insert_or_throw(&mut options_builder, given, option_value);
            }
        } else {
            let option_value = find_starlark_option_value(build_options, canonical);
            insert_or_throw(&mut options_builder, given, option_value);
        }
    }

    let remaining_inputs: Vec<&str> = inputs_canonicalized_to_given
        .iter()
        .map(|(_, g)| g.as_str())
        .filter(|g| !options_builder.contains_key(*g))
        .collect();
    if !remaining_inputs.is_empty() {
        return Err(ValidationException(format!(
            "transition inputs [{}] do not correspond to valid settings",
            remaining_inputs.join(", ")
        )));
    }
    Ok(options_builder)
}

/// `ImmutableMap.Builder.put` + `buildOrThrow`.
fn insert_or_throw(map: &mut IndexMap<String, JavaObject>, key: &str, value: JavaObject) {
    if map.insert(key.to_string(), value).is_some() {
        panic!(
            "IllegalArgumentException: Multiple entries with same key: {}",
            key
        );
    }
}

// Java: com/google/devtools/build/lib/analysis/starlark/FunctionTransitionUtil.java:541 FunctionTransitionUtil.findNativeOptionValue
fn find_native_option_value(
    build_options: &BuildOptions,
    option_info_map: &IndexMap<String, OptionInfo>,
    flags_aliases: &IndexMap<String, Label>,
    setting: &Label,
) -> Option<JavaObject> {
    let option_name = setting.get_name();
    if let Some(starlark_flag) = flags_aliases.get(option_name) {
        // If the setting is an alias to a starlark option, use the starlark option value.
        return Some(find_starlark_option_value(build_options, starlark_flag));
    }
    let option_info = option_info_map.get(option_name)?;
    let options = build_options
        .get_dyn(option_info.get_option_class())
        .expect("present");
    // Get the raw value to avoid the default handling for null values.
    let option_value = option_info.get_definition().get_raw_value(options);
    // convert nulls here b/c ImmutableMap bans null values
    Some(JavaObject::from_option_value(option_value.as_ref()))
}

// Java: com/google/devtools/build/lib/analysis/starlark/FunctionTransitionUtil.java:544 FunctionTransitionUtil.findStarlarkOptionValue
fn find_starlark_option_value(build_options: &BuildOptions, setting: &Label) -> JavaObject {
    match build_options.get_starlark_options().get(setting) {
        None => JavaObject::Null,
        Some(v) => starlark_option_to_java(v),
    }
}

/// The object a transition reads for a Starlark option (`Starlark.fromJava` then turns Java
/// lists and sets into Starlark lists).
fn starlark_option_to_java(v: &StarlarkOptionValue) -> JavaObject {
    match v {
        StarlarkOptionValue::Bool(b) => JavaObject::Bool(*b),
        StarlarkOptionValue::Int(i) => JavaObject::Int(*i),
        StarlarkOptionValue::Str(s) => JavaObject::Str(s.clone()),
        StarlarkOptionValue::Label(l) => JavaObject::Label(l.clone()),
        // PORT: Java hands the FeatureFlagValue object to Starlark.fromJava, which rejects a
        // non-Starlark class; not reached by any ported caller.
        StarlarkOptionValue::FeatureFlag(v) => blaze_util::unported!(
            "FunctionTransitionUtil.findStarlarkOptionValue",
            "feature flag value {} as a transition input",
            v
        ),
        StarlarkOptionValue::List(l) => {
            let items = l.iter().map(starlark_option_to_java).collect();
            match l.class() {
                JavaCollectionClass::Tuple => JavaObject::Tuple(items),
                JavaCollectionClass::List => JavaObject::List(items),
                JavaCollectionClass::StarlarkList => JavaObject::StarlarkList(items),
                // Starlark.fromJava turns a java.util.Set into a StarlarkSet (a StarlarkSet is
                // passed through).
                JavaCollectionClass::Set | JavaCollectionClass::StarlarkSet => {
                    JavaObject::Set(items)
                }
            }
        }
    }
}

/// The `BuildOptions` form of a value a transition function returned: Java stores the Starlark
/// object itself (`StarlarkList`, `Tuple`, `String`, `StarlarkInt`, `Boolean`, `Label`).
fn java_to_starlark_option(v: &JavaObject) -> Result<StarlarkOptionValue, ValidationException> {
    Ok(match v {
        JavaObject::Bool(b) => StarlarkOptionValue::Bool(*b),
        JavaObject::Int(i) => StarlarkOptionValue::Int(*i),
        JavaObject::Str(s) => StarlarkOptionValue::Str(s.clone()),
        JavaObject::Label(l) => StarlarkOptionValue::Label(l.clone()),
        JavaObject::List(l) | JavaObject::StarlarkList(l) => StarlarkOptionValue::starlark_list(
            l.iter()
                .map(java_to_starlark_option)
                .collect::<Result<_, _>>()?,
        ),
        JavaObject::Tuple(l) => StarlarkOptionValue::tuple(
            l.iter()
                .map(java_to_starlark_option)
                .collect::<Result<_, _>>()?,
        ),
        JavaObject::Set(l) => StarlarkOptionValue::starlark_set(
            l.iter()
                .map(java_to_starlark_option)
                .collect::<Result<_, _>>()?,
        ),
        other => blaze_util::unported!(
            "FunctionTransitionUtil.applyTransition",
            "Starlark option value {:?}",
            other
        ),
    })
}

// Java: com/google/devtools/build/lib/analysis/starlark/FunctionTransitionUtil.java:156 FunctionTransitionUtil.applyTransition
/// Apply the transition dictionary to the build option, using optionInfoMap to look up the
/// option info.
fn apply_transition(
    from_options: &BuildOptions,
    new_values: &IndexMap<Label, JavaObject>,
    option_info_map: &IndexMap<String, OptionInfo>,
    starlark_transition: &StarlarkDefinedConfigTransition,
) -> Result<BuildOptions, ValidationException> {
    // toOptions being null means the transition hasn't changed anything. We avoid preemptively
    // cloning it from fromOptions since options cloning is an expensive operation.
    let mut to_options: Option<BuildOptions> = None;
    // The names of options (Starlark + native) that are different after this transition and
    // must be added to "affected by Starlark transition"
    let mut converted_affected_options: BTreeSet<String> = BTreeSet::new();
    // Starlark options that are different after this transition.
    let mut changed_starlark_options: IndexMap<Label, StarlarkOptionValue> = IndexMap::new();
    for (option_key, option_value) in new_values {
        if option_key.get_package_identifier() != &*COMMAND_LINE_OPTION_PACKAGE_IDENTIFIER {
            // The transition changes a Starlark option.
            let old_value = from_options.get_starlark_options().get(option_key);
            let mut option_value = option_value.clone();
            if let Some(StarlarkOptionValue::Label(_)) = old_value {
                // If this is a label-typed build setting, we need to convert the provided new
                // value into a Label object.
                match &option_value {
                    JavaObject::Str(s) => {
                        let parsed = Label::parse_with_package_context(
                            s,
                            starlark_transition.get_package_context(),
                        )
                        .map_err(|e| {
                            ValidationException(format!(
                                "Error parsing value for option '{}': {}",
                                option_key,
                                e.get_message()
                            ))
                        })?;
                        option_value = JavaObject::Label(parsed);
                    }
                    JavaObject::Label(_) => {}
                    other => {
                        return Err(ValidationException(format!(
                            "Invalid value type for option '{}': want label, got {}",
                            option_key,
                            other.starlark_type()
                        )));
                    }
                }
            }
            let mut new_value = java_to_starlark_option(&option_value)?;
            if let Some(StarlarkOptionValue::List(old_list)) = old_value
                && old_list.is_java_set()
            {
                // If this is a set-typed build setting, for backwards compatibility, if the
                // provided value is a List, we need to convert it to a Set.
                let as_set = match &new_value {
                    // ImmutableSet.copyOf((List<?>) optionValue)
                    StarlarkOptionValue::List(new_list) if new_list.is_java_list() => {
                        Some(StarlarkOptionValue::set(new_list.items().to_vec()))
                    }
                    StarlarkOptionValue::List(_) => None,
                    _ => {
                        return Err(ValidationException(format!(
                            "Invalid value type for option '{}': want set, got {}",
                            option_key,
                            option_value.starlark_type()
                        )));
                    }
                };
                if let Some(as_set) = as_set {
                    new_value = as_set;
                }
            }
            // Java: !Objects.equals(oldValue, optionValue)
            if !StarlarkOptionValue::java_objects_equals(old_value, Some(&new_value)) {
                changed_starlark_options.insert(option_key.clone(), new_value);
                converted_affected_options.insert(option_key.to_string());
            }
        } else {
            // The transition changes a native option.
            let option_name = option_key.get_name();
            let option_info = option_info_map.get(option_name).expect("validated");
            let def = option_info.get_definition();

            // Convert NoneType to null; StarlarkInt to Integer; lists and maps to Java-native.
            let option_value = option_value.clone();
            let converted_value: Option<OptionValue>;
            let field = def.field_info();
            let parse_err = |e: OptionsParsingException| {
                ValidationException(format!(
                    "OptionsParsingError for option '{}': {}",
                    option_name,
                    e.get_message()
                ))
            };
            // Java: optionValue instanceof List (a Starlark tuple is a java.util.List too).
            if field.is_list
                && matches!(
                    option_value,
                    JavaObject::List(_) | JavaObject::StarlarkList(_) | JavaObject::Tuple(_)
                )
            {
                let (JavaObject::List(option_value_as_list)
                | JavaObject::StarlarkList(option_value_as_list)
                | JavaObject::Tuple(option_value_as_list)) = &option_value
                else {
                    unreachable!()
                };
                // This is possible with Starlark code like
                // "{ //command_line_option:foo: ["a", "b"] }". We convert its elements with
                // .toString() with a "," separator, which happens to work for most
                // implementations.
                if option_value_as_list.is_empty() {
                    converted_value = Some(OptionValue::list(Vec::new()));
                } else if !def.allows_multiple() {
                    let joined: Vec<String> = option_value_as_list
                        .iter()
                        .map(|element| match element {
                            JavaObject::Label(label) => label.get_unambiguous_canonical_form(),
                            other => other.java_to_string(),
                        })
                        .collect();
                    converted_value = def
                        .get_converter()
                        .convert(
                            Some(&joined.join(",")),
                            Some(&package_conversion_context(starlark_transition)
                                as &blaze_options::ConversionContext),
                        )
                        .map_err(parse_err)?;
                } else {
                    let mut value_builder: Vec<OptionValue> = Vec::new();
                    for e in option_value_as_list {
                        let converted = def
                            .get_converter()
                            .convert(
                                Some(&e.java_to_string()),
                                Some(&package_conversion_context(starlark_transition)
                                    as &blaze_options::ConversionContext),
                            )
                            .map_err(parse_err)?;
                        match converted {
                            Some(c) => match c.as_list() {
                                Some(list) => value_builder.extend(list.iter().cloned()),
                                None => value_builder.push(c),
                            },
                            None => panic!("NullPointerException: ImmutableList.Builder.add(null)"),
                        }
                    }
                    converted_value = Some(OptionValue::list(value_builder));
                }
            } else if field.is_list && matches!(option_value, JavaObject::Null) {
                return Err(ValidationException(format!(
                    "'None' value not allowed for List-type option '{}'. Please use '[]' instead if trying to set option to empty value.",
                    option_name
                )));
            } else if let JavaObject::Map(entries) = &option_value
                && field.java_type_name == "java.util.Map"
            {
                // Java: `optionValue = ImmutableMap.copyOf(dict)`, then
                // `def.getType().isInstance(optionValue)` holds for a Map-typed option and the
                // map is stored as it is.
                converted_value = Some(map_option_value(field, entries));
            } else if matches!(option_value, JavaObject::Null)
                || is_instance(option_info, &option_value)
            {
                converted_value = option_value.to_option_value();
            } else if let JavaObject::Str(s) = &option_value {
                converted_value = def
                    .get_converter()
                    .convert(
                        Some(s),
                        Some(&package_conversion_context(starlark_transition)
                            as &blaze_options::ConversionContext),
                    )
                    .map_err(parse_err)?;
            } else {
                return Err(ValidationException(format!(
                    "Invalid value type for option '{}'",
                    option_name
                )));
            }

            let old_value = def.get_raw_value(
                from_options
                    .get_dyn(option_info.get_option_class())
                    .unwrap(),
            );
            // Java: !Objects.equals(oldValue, convertedValue)
            if !option_values_java_equal(old_value.as_ref(), converted_value.as_ref()) {
                let to = to_options.get_or_insert_with(|| from_options.clone());
                let target = to
                    .get_dyn_mut(option_info.get_option_class())
                    .expect("present");
                def.set_value(target, converted_value).map_err(|_| {
                    ValidationException(format!(
                        "IllegalArgumentError for option '{}': {}",
                        option_name, "value does not match the option type"
                    ))
                })?;
                converted_affected_options.insert(option_key.to_string());
            }
        }
    }

    if to_options.is_none() && changed_starlark_options.is_empty() {
        return Ok(from_options.clone());
    }
    // Note that rebuilding also calls FragmentOptions.getNormalized() to guarantee --define,
    // --features, and similar flags are consistently ordered.
    let base = match to_options {
        Some(o) => o,
        None => from_options.clone(),
    };
    let mut to_options = BuildOptions::builder()
        .merge(&base)
        .add_starlark_options(changed_starlark_options)
        .build();
    if starlark_transition.is_for_analysis_testing() {
        // We need to record every time we change a configuration option.
        converted_affected_options
            .insert("//command_line_option:evaluating for analysis test".to_string());
        to_options
            .get_mut::<CoreOptions>()
            .expect("CoreOptions")
            .evaluating_for_analysis_test = true;
    }

    let is_exec_transition = starlark_transition.is_exec_transition();
    let core_options = to_options.get_mut::<CoreOptions>().expect("CoreOptions");
    if !is_exec_transition
        && core_options.output_directory_naming_scheme
            == CoreOptionsOutputDirectoryNamingScheme::LEGACY
    {
        // The exec transition uses its own logic in ExecutionTransitionFactory.
        update_affected_by_starlark_transition(core_options, &converted_affected_options);
    }
    Ok(to_options)
}

/// `ImmutableMap.copyOf(dict)` stored in a field whose Java type is `Map`. Java stores the map
/// whatever the classes of its keys and values (the generic type is erased); the Rust field has
/// a concrete type, so only entries of the field's element classes are carried over.
// Java: com/google/devtools/build/lib/analysis/starlark/FunctionTransitionUtil.java:156 FunctionTransitionUtil.applyTransition (the Map branch, lines 678-681 and 724-725)
fn map_option_value(
    field: &blaze_options::FieldInfo,
    entries: &[(JavaObject, JavaObject)],
) -> OptionValue {
    use blaze_optdefs::common::test_timeout::TestTimeoutMap;
    use blaze_optdefs::fragment::converters::LabelMap;
    if field.rust_type_name == std::any::type_name::<LabelMap>() {
        // `JavaOptions.bytecodeOptimizers`: `Map<String, Label>`.
        let mut map: Vec<(String, Option<Label>)> = Vec::with_capacity(entries.len());
        for (k, v) in entries {
            match (k, v) {
                (JavaObject::Str(k), JavaObject::Label(l)) => {
                    map.push((k.clone(), Some(l.clone())))
                }
                _ => blaze_util::unported!(
                    "FunctionTransitionUtil.applyTransition",
                    "a dict entry that is not String=Label for a Map<String, Label> option \
                     (Java stores it by heap pollution)"
                ),
            }
        }
        return OptionValue::new(LabelMap(map));
    }
    if field.rust_type_name == std::any::type_name::<TestTimeoutMap>() {
        // `TestOptions.testTimeout`: `Map<TestTimeout, Duration>`. Starlark has no `TestTimeout`
        // or `Duration` values, so only the empty dict has the field's element classes.
        if entries.is_empty() {
            return OptionValue::new(TestTimeoutMap::default());
        }
        blaze_util::unported!(
            "FunctionTransitionUtil.applyTransition",
            "a non-empty dict for a Map<TestTimeout, Duration> option (Java stores it by heap \
             pollution)"
        );
    }
    blaze_util::unported!(
        "FunctionTransitionUtil.applyTransition",
        "a dict for a Map-typed option of another field type"
    )
}

/// `Objects.equals(oldValue, convertedValue)` for two option values. `Map.equals` ignores the
/// iteration order; the Rust `LabelMap` (an ordered entry list) compares in order, so it is
/// compared as a map here.
fn option_values_java_equal(a: Option<&OptionValue>, b: Option<&OptionValue>) -> bool {
    use blaze_optdefs::fragment::converters::LabelMap;
    if let (Some(a), Some(b)) = (a, b)
        && let (Some(a), Some(b)) = (a.downcast_ref::<LabelMap>(), b.downcast_ref::<LabelMap>())
    {
        return a.0.len() == b.0.len() && a.0.iter().all(|e| b.0.contains(e));
    }
    a == b
}

/// `def.getType().isInstance(optionValue)` (plus the `int`/`boolean` primitive cases).
fn is_instance(option_info: &OptionInfo, value: &JavaObject) -> bool {
    // PORT: Rust option values are type-erased; test whether the value fits the field by setting
    // it on a scratch instance of the options class.
    let Some(v) = value.to_option_value_if_typed() else {
        return false;
    };
    let mut scratch = option_info.get_option_class().get_default();
    option_info
        .get_definition()
        .set_value(scratch.as_mut(), Some(v))
        .is_ok()
}

impl JavaObject {
    /// The option value this object is when `isInstance` can hold (not for Strings that need a
    /// converter: a `String` is only an instance of `String` fields, which the scratch set
    /// checks).
    fn to_option_value_if_typed(&self) -> Option<OptionValue> {
        match self {
            JavaObject::Null
            | JavaObject::List(_)
            | JavaObject::StarlarkList(_)
            | JavaObject::Tuple(_)
            | JavaObject::Map(_)
            | JavaObject::Set(_) => None,
            other => other.to_option_value(),
        }
    }
}

/// `starlarkTransition.getPackageContext()` as a converter conversion context.
fn package_conversion_context(
    starlark_transition: &StarlarkDefinedConfigTransition,
) -> blaze_cmdline::LabelPackageContext {
    starlark_transition.get_package_context().clone()
}

// Java: com/google/devtools/build/lib/analysis/starlark/FunctionTransitionUtil.java:798 FunctionTransitionUtil.getAffectedByStarlarkTransitionViaDiff
/// Return different options in "affected by Starlark transition" form.
pub fn get_affected_by_starlark_transition_via_diff(
    to_options: &BuildOptions,
    baseline_options: &BuildOptions,
) -> IndexSet<String> {
    if to_options == baseline_options {
        return IndexSet::new();
    }
    let diff = OptionsDiff::diff(to_options, baseline_options);
    let mut result: IndexSet<String> = diff
        .get_first()
        .keys()
        .map(|option| format!("{}{}", COMMAND_LINE_OPTION_PREFIX, option.get_option_name()))
        .collect();
    result.extend(
        diff.get_changed_starlark_options()
            .iter()
            .map(|l| l.to_string()),
    );
    result
}

// Java: com/google/devtools/build/lib/analysis/starlark/FunctionTransitionUtil.java:790 FunctionTransitionUtil.updateAffectedByStarlarkTransition
/// Extend the global build config affectedByStarlarkTransition, by adding any new option names
/// from changedOptions.
pub fn update_affected_by_starlark_transition<'a>(
    build_config_options: &mut CoreOptions,
    changed_options: impl IntoIterator<Item = &'a String>,
) {
    let changed: Vec<&String> = changed_options.into_iter().collect();
    if changed.is_empty() {
        return;
    }
    let mut mutable_copy_to_update: BTreeSet<JavaOrderedString> = build_config_options
        .affected_by_starlark_transition
        .iter()
        .map(|s| JavaOrderedString(s.clone()))
        .collect();
    mutable_copy_to_update.extend(changed.into_iter().map(|s| JavaOrderedString(s.clone())));
    build_config_options.affected_by_starlark_transition =
        mutable_copy_to_update.into_iter().map(|s| s.0).collect();
}

#[derive(PartialEq, Eq)]
struct JavaOrderedString(String);
impl Ord for JavaOrderedString {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        crate::build_options::java_compare(&self.0, &other.0)
    }
}
impl PartialOrd for JavaOrderedString {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

// -------------------------------------------------------------------------------------------
// The exec transition.

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttributeTransitionProvider.java:141 FunctionSplitTransition.split
/// Applies a Starlark attribute transition provider's transition (the exec transition provider
/// when `is_exec_transition_provider`). Returns `{"error": clone}` on errors.
pub fn function_split_transition_split(
    build_options: &BuildOptions,
    starlark_transition: &StarlarkDefinedConfigTransition,
    allow_immutable_flag_changes: bool,
    is_exec_transition_provider: bool,
    semantics: &Arc<StarlarkSemantics>,
    event_handler: &dyn EventHandler,
) -> IndexMap<String, BuildOptions> {
    match apply_and_validate(
        build_options,
        starlark_transition,
        allow_immutable_flag_changes,
        is_exec_transition_provider,
        semantics,
        event_handler,
    ) {
        Some(res) => res,
        None => {
            let mut m = IndexMap::new();
            m.insert("error".to_string(), build_options.clone());
            m
        }
    }
}

// Java: com/google/devtools/build/lib/analysis/config/ExecutionTransitionFactory.java:154 ExecTransitionFinalizer.FRAGMENTS
pub fn exec_transition_required_fragments()
-> Vec<crate::fragment_options_class::FragmentOptionsClass> {
    vec![&CORE_OPTIONS, &PLATFORM_OPTIONS]
}

// Java: com/google/devtools/build/lib/analysis/config/ExecutionTransitionFactory.java:192 ExecTransitionFinalizer.patch
/// The complete exec transition: calls the Starlark exec transition (as a
/// `StarlarkExecTransitionProvider`: immutable flag changes allowed), then runs the finalizer
/// logic that has to be handled natively.
pub fn exec_transition_patch(
    execution_platform: Option<&Label>,
    main_transition: &StarlarkDefinedConfigTransition,
    options: &BuildOptionsView,
    semantics: &Arc<StarlarkSemantics>,
    event_handler: &dyn EventHandler,
) -> BuildOptions {
    let Some(execution_platform) = execution_platform else {
        // No execution platform is known, so don't change anything.
        return options.underlying().clone();
    };

    // SplitTransition.apply: the split must produce exactly one entry here.
    let split = function_split_transition_split(
        options.underlying(),
        main_transition,
        /* allow_immutable_flag_changes= */ true,
        /* is_exec_transition_provider= */ true,
        semantics,
        event_handler,
    );
    if split.len() != 1 {
        panic!(
            "IllegalArgumentException: expected one element but was: <{}>",
            split.keys().cloned().collect::<Vec<_>>().join(", ")
        );
    }
    let mut exec_options = split.into_iter().next().unwrap().1;

    // Set the target to the saved execution platform if there is one.
    if let Some(platform_options) = exec_options.get_mut::<PlatformOptions>() {
        platform_options.platforms = vec![Foreign(execution_platform.clone())];
    }

    // Remove any FeatureFlags that were set.
    let feature_flags: Vec<Label> = exec_options
        .get_starlark_options()
        .iter()
        .filter(|(_, value)| matches!(value, StarlarkOptionValue::FeatureFlag(_)))
        .map(|(key, _)| key.clone())
        .collect();

    let mut result = exec_options;
    if !feature_flags.is_empty() {
        let mut result_builder = result.to_builder();
        for flag in &feature_flags {
            result_builder.remove_starlark_option(flag);
        }
        result = result_builder.build();
    }

    let underlying_core = options
        .underlying()
        .get::<CoreOptions>()
        .expect("CoreOptions")
        .clone();
    {
        let core_options = result.get_mut::<CoreOptions>().expect("CoreOptions");
        core_options.command_line_flag_aliases = underlying_core.command_line_flag_aliases.clone();
    }
    let scheme = result
        .get::<CoreOptions>()
        .unwrap()
        .exec_configuration_distinguisher_scheme;
    match scheme {
        CoreOptionsExecConfigurationDistinguisherScheme::LEGACY => {
            result.get_mut::<CoreOptions>().unwrap().platform_suffix = Some(format!(
                "exec-{:X}",
                crate::output_path_mnemonic_computer::java_string_hash_code(
                    &execution_platform.get_canonical_form()
                ) as u32
            ));
        }
        CoreOptionsExecConfigurationDistinguisherScheme::FULL_HASH => {
            // Java: com/google/devtools/build/lib/analysis/config/ExecutionTransitionFactory.java:235 ExecutionTransitionFactory.ExecTransitionFinalizer.patch
            result.get_mut::<CoreOptions>().unwrap().platform_suffix = Some(String::new());
            // execOptions creation above made a clone, which will have a fresh hashCode.
            // PORT: the checksum is computed on a clone, which has no cached checksum
            // (Java: `result` is a fresh clone at this point).
            // Java: com/google/devtools/build/lib/analysis/config/BuildOptions.java:284 BuildOptions.hashCode
            let full_hash =
                31i32.wrapping_add(crate::output_path_mnemonic_computer::java_string_hash_code(
                    result.clone().checksum(),
                ));
            result.get_mut::<CoreOptions>().unwrap().platform_suffix =
                Some(format!("exec-{:X}", full_hash as u32));
            // Previous call to hashCode irreparably locked in state so must clone to refresh
            // since options mutated after that.
            result = result.clone();
        }
        CoreOptionsExecConfigurationDistinguisherScheme::DIFF_TO_AFFECTED => {
            // Setting platform_suffix here should not be necessary for correctness but done for
            // user clarity.
            result.get_mut::<CoreOptions>().unwrap().platform_suffix = Some("exec".to_string());
            let diff = get_affected_by_starlark_transition_via_diff(&result, options.underlying());
            update_affected_by_starlark_transition(result.get_mut::<CoreOptions>().unwrap(), &diff);
            // Previous call to diff irreparably locked in state so must clone to refresh.
            // PORT: Java's assignments after this switch go to the `coreOptions` object of the
            // pre-clone options, which is discarded; the returned clone keeps the values set
            // above. Returning here reproduces that.
            return result.clone();
        }
        CoreOptionsExecConfigurationDistinguisherScheme::OFF => {
            // else if OFF just mark that we are now in an exec transition
            result.get_mut::<CoreOptions>().unwrap().platform_suffix = Some("exec".to_string());
        }
    }
    let core_options = result.get_mut::<CoreOptions>().expect("CoreOptions");
    core_options.affected_by_starlark_transition =
        underlying_core.affected_by_starlark_transition.clone();
    core_options.execution_info_modifier = underlying_core.execution_info_modifier.clone();
    core_options.override_platform_cpu_name = underlying_core.override_platform_cpu_name.clone();
    core_options.disabled_select_options = underlying_core.disabled_select_options.clone();
    core_options.incompatible_target_cpu_from_platform =
        underlying_core.incompatible_target_cpu_from_platform;
    result
}

// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:1541 SkyframeExecutor.adjustForExec
/// Adjusts the baseline options for the exec transition.
pub fn adjust_for_exec(
    baseline_options: &BuildOptions,
    starlark_exec_transition: &StarlarkDefinedConfigTransition,
    new_platform: Option<&Label>,
    semantics: &Arc<StarlarkSemantics>,
    event_handler: &dyn EventHandler,
) -> BuildOptions {
    // A null executionPlatform actually skips transition application so need some value here
    // when not overriding the platform. It is safe to supply some fake value here (as long as it
    // is constant) since the baseline should never be used to actually construct an action or do
    // toolchain resolution.
    let fake;
    let platform = match new_platform {
        Some(p) => p,
        None => {
            fake = Label::parse_canonical_unchecked(
                "//this_is_a_faked_exec_platform_for_blaze_internals",
            );
            &fake
        }
    };
    let view = crate::transitions::restrict(exec_transition_required_fragments(), baseline_options);
    exec_transition_patch(
        Some(platform),
        starlark_exec_transition,
        &view,
        semantics,
        event_handler,
    )
}
