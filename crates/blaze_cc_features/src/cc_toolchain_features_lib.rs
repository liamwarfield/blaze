//! Port of `com.google.devtools.build.lib.rules.cpp.CcToolchainFeaturesLib`: converts the
//! `CcToolchainConfigInfo` structs created by rules_cc's `cc_toolchain_config_lib.bzl` into native
//! features, action configs and artifact name patterns.

use std::sync::{Arc, LazyLock};

use blaze_vfs::PathFragment;
use regex::Regex;

use crate::actions::{OS, path_fragment_create_for_os};
use crate::artifact_category::ArtifactCategory;
use crate::cc_toolchain_features::{
    ActionConfig, EnvEntry, EnvSet, Feature, Flag, FlagGroup, FlagSet, PathOrigin, StringSet, Tool,
    VariableWithValue, WithFeatureSet, WithFeatureSetSet,
};
use crate::cc_toolchain_variables::StringValueParser;
use crate::errors::EvalException;
use crate::starlark_info::{StarlarkInfo, StarlarkInfoValue, sequence_cast};

// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeaturesLib.java:56 CcToolchainFeaturesLib.infoError
fn info_error(info: &StarlarkInfo, message: String) -> EvalException {
    EvalException::new(format!(
        "in {} instantiated at {}: {}",
        info.provider_printable_name, info.creation_location, message
    ))
}

/// Checks whether the `StarlarkInfo` is of the required type.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeaturesLib.java:65 CcToolchainFeaturesLib.checkRightProviderType
fn check_right_provider_type(provider: &StarlarkInfo, type_: &str) -> Result<(), EvalException> {
    let provider_type = match get_value_or_null(provider, "type_name") {
        Some(StarlarkInfoValue::String(s)) => s.clone(),
        Some(_) => panic!("ClassCastException: type_name is not a String"),
        None => provider.provider_printable_name.clone(),
    };
    if !matches!(provider.get_value("type_name"), Some(StarlarkInfoValue::String(s)) if s == type_)
    {
        return Err(info_error(
            provider,
            format!("Expected object of type '{type_}', received '{provider_type}'."),
        ));
    }
    Ok(())
}

// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeaturesLib.java:77 CcToolchainFeaturesLib.getValueOrNull
fn get_value_or_null<'a>(x: &'a StarlarkInfo, name: &str) -> Option<&'a StarlarkInfoValue> {
    x.get_value(name)
}

/// The `Class<T>` argument of the field getters.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FieldClass {
    String,
    Boolean,
    StarlarkInfo,
    Artifact,
}

impl FieldClass {
    /// Java: `clazz.getName()`.
    fn java_name(self) -> &'static str {
        match self {
            FieldClass::String => "java.lang.String",
            FieldClass::Boolean => "java.lang.Boolean",
            FieldClass::StarlarkInfo => "com.google.devtools.build.lib.packages.StarlarkInfo",
            FieldClass::Artifact => "com.google.devtools.build.lib.actions.Artifact",
        }
    }

    fn is_instance(self, v: &StarlarkInfoValue) -> bool {
        matches!(
            (self, v),
            (FieldClass::String, StarlarkInfoValue::String(_))
                | (FieldClass::Boolean, StarlarkInfoValue::Bool(_))
                | (FieldClass::StarlarkInfo, StarlarkInfoValue::Info(_))
                | (FieldClass::Artifact, StarlarkInfoValue::Artifact(_))
        )
    }
}

// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeaturesLib.java:509 CcToolchainFeaturesLib.getFieldFromStarlarkProvider
fn get_field_from_starlark_provider<'a>(
    provider: &'a StarlarkInfo,
    field_name: &str,
    clazz: FieldClass,
    mandatory: bool,
) -> Result<Option<&'a StarlarkInfoValue>, EvalException> {
    let Some(obj) = provider.get_value(field_name) else {
        if mandatory {
            return Err(info_error(
                provider,
                format!("Missing mandatory field '{field_name}'"),
            ));
        }
        return Ok(None);
    };
    if clazz.is_instance(obj) {
        return Ok(Some(obj));
    }
    if matches!(obj, StarlarkInfoValue::None) {
        return Ok(None);
    }
    Err(info_error(
        provider,
        format!(
            "Field '{}' is not of '{}' type.",
            field_name,
            clazz.java_name()
        ),
    ))
}

fn get_mandatory_string(p: &StarlarkInfo, field: &str) -> Result<Option<String>, EvalException> {
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeaturesLib.java:504 CcToolchainFeaturesLib.getMandatoryFieldFromStarlarkProvider
    Ok(
        match get_field_from_starlark_provider(p, field, FieldClass::String, true)? {
            Some(StarlarkInfoValue::String(s)) => Some(s.clone()),
            _ => None,
        },
    )
}

fn get_optional_string(p: &StarlarkInfo, field: &str) -> Result<Option<String>, EvalException> {
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeaturesLib.java:499 CcToolchainFeaturesLib.getOptionalFieldFromStarlarkProvider
    Ok(
        match get_field_from_starlark_provider(p, field, FieldClass::String, false)? {
            Some(StarlarkInfoValue::String(s)) => Some(s.clone()),
            _ => None,
        },
    )
}

fn get_mandatory_bool(p: &StarlarkInfo, field: &str) -> Result<Option<bool>, EvalException> {
    Ok(
        match get_field_from_starlark_provider(p, field, FieldClass::Boolean, true)? {
            Some(StarlarkInfoValue::Bool(b)) => Some(*b),
            _ => None,
        },
    )
}

/// Returns a list of strings from a field of a `StarlarkInfo`.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeaturesLib.java:529 CcToolchainFeaturesLib.getStringListFromStarlarkProviderField
fn get_string_list_from_starlark_provider_field(
    provider: &StarlarkInfo,
    field_name: &str,
) -> Result<Vec<String>, EvalException> {
    match get_value_or_null(provider, field_name) {
        None => Ok(Vec::new()),
        Some(v) => Ok(sequence_cast(v, "string", field_name, true)?
            .iter()
            .map(|e| match e {
                StarlarkInfoValue::String(s) => s.clone(),
                _ => unreachable!(),
            })
            .collect()),
    }
}

/// Returns a set of strings from a field of a `StarlarkInfo`.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeaturesLib.java:538 CcToolchainFeaturesLib.getStringSetFromStarlarkProviderField
fn get_string_set_from_starlark_provider_field(
    provider: &StarlarkInfo,
    field_name: &str,
) -> Result<StringSet, EvalException> {
    Ok(StringSet::of(get_string_list_from_starlark_provider_field(
        provider, field_name,
    )?))
}

/// Returns a list of StarlarkInfo providers from a field of a `StarlarkInfo`.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeaturesLib.java:547 CcToolchainFeaturesLib.getStarlarkProviderListFromStarlarkField
fn get_starlark_provider_list_from_starlark_field(
    provider: &StarlarkInfo,
    field_name: &str,
) -> Result<Vec<Arc<StarlarkInfo>>, EvalException> {
    match get_value_or_null(provider, field_name) {
        None => Ok(Vec::new()),
        Some(v) => Ok(sequence_cast(v, "struct", field_name, true)?
            .iter()
            .map(|e| match e {
                StarlarkInfoValue::Info(i) => i.clone(),
                _ => unreachable!(),
            })
            .collect()),
    }
}

static NAME_PATTERN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[_a-z0-9+\-\.]*$").unwrap());

/// Creates a [`Feature`] from a `StarlarkInfo`.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeaturesLib.java:87 CcToolchainFeaturesLib.featureFromStarlark
pub fn feature_from_starlark(feature_struct: &StarlarkInfo) -> Result<Feature, EvalException> {
    check_right_provider_type(feature_struct, "feature")?;
    let name = get_mandatory_string(feature_struct, "name")?;
    let enabled = get_mandatory_bool(feature_struct, "enabled")?;
    let name = match name {
        Some(name)
            if !(name.is_empty() && !enabled.unwrap_or_else(|| panic!("NullPointerException"))) =>
        {
            name
        }
        _ => {
            return Err(info_error(
                feature_struct,
                "A feature must either have a nonempty 'name' field or be enabled.".to_string(),
            ));
        }
    };
    let enabled = enabled.unwrap_or_else(|| panic!("NullPointerException"));

    if !NAME_PATTERN.is_match(&name) {
        return Err(info_error(
            feature_struct,
            format!(
                "A feature's name must consist solely of lowercase ASCII letters, digits, '.', \
                 '_', '+', and '-', got '{name}'"
            ),
        ));
    }

    let mut flag_set_builder = Vec::new();
    let flag_sets = get_starlark_provider_list_from_starlark_field(feature_struct, "flag_sets")?;
    for flag_set_object in &flag_sets {
        let flag_set = flag_set_from_starlark(flag_set_object, /* action_name= */ None)?;
        if flag_set.actions.is_empty() {
            return Err(info_error(
                flag_set_object,
                "A flag_set that belongs to a feature must have nonempty 'actions' parameter."
                    .to_string(),
            ));
        }
        flag_set_builder.push(flag_set);
    }

    let mut env_set_builder = Vec::new();
    let env_sets = get_starlark_provider_list_from_starlark_field(feature_struct, "env_sets")?;
    for env_set in &env_sets {
        env_set_builder.push(env_set_from_starlark(env_set)?);
    }

    let mut requires_builder = Vec::new();
    let requires = get_starlark_provider_list_from_starlark_field(feature_struct, "requires")?;
    for feature_set_struct in &requires {
        // getValue() may be null
        if !matches!(feature_set_struct.get_value("type_name"),
            Some(StarlarkInfoValue::String(s)) if s == "feature_set")
        {
            return Err(info_error(
                feature_struct,
                "expected object of type 'feature_set'.".to_string(),
            ));
        }
        let feature_set =
            get_string_set_from_starlark_provider_field(feature_set_struct, "features")?;
        requires_builder.push(feature_set);
    }

    let implies = get_string_list_from_starlark_provider_field(feature_struct, "implies")?;

    let provides = get_string_list_from_starlark_provider_field(feature_struct, "provides")?;

    Ok(Feature::new(
        name,
        flag_set_builder,
        env_set_builder,
        enabled,
        requires_builder,
        implies,
        provides,
    ))
}

/// Creates a Pair(name, value) that represents a `CrosstoolConfig.MakeVariable`.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeaturesLib.java:160 CcToolchainFeaturesLib.makeVariableFromStarlark
pub fn make_variable_from_starlark(
    make_variable_struct: &StarlarkInfo,
) -> Result<(String, String), EvalException> {
    check_right_provider_type(make_variable_struct, "make_variable")?;
    let name = get_mandatory_string(make_variable_struct, "name")?;
    let value = get_mandatory_string(make_variable_struct, "value")?;
    let Some(name) = name.filter(|n| !n.is_empty()) else {
        return Err(info_error(
            make_variable_struct,
            "'name' parameter of make_variable must be a nonempty string.".to_string(),
        ));
    };
    let Some(value) = value.filter(|v| !v.is_empty()) else {
        return Err(info_error(
            make_variable_struct,
            "'value' parameter of make_variable must be a nonempty string.".to_string(),
        ));
    };
    Ok((name, value))
}

/// Creates a Pair(name, path) that represents a `CrosstoolConfig.ToolPath`.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeaturesLib.java:182 CcToolchainFeaturesLib.toolPathFromStarlark
pub fn tool_path_from_starlark(
    tool_path_struct: &StarlarkInfo,
) -> Result<(String, String), EvalException> {
    check_right_provider_type(tool_path_struct, "tool_path")?;
    let name = get_mandatory_string(tool_path_struct, "name")?;
    let path = get_mandatory_string(tool_path_struct, "path")?;
    let Some(name) = name.filter(|n| !n.is_empty()) else {
        return Err(info_error(
            tool_path_struct,
            "'name' parameter of tool_path must be a nonempty string.".to_string(),
        ));
    };
    let Some(path) = path.filter(|p| !p.is_empty()) else {
        return Err(info_error(
            tool_path_struct,
            "'path' parameter of tool_path must be a nonempty string.".to_string(),
        ));
    };
    Ok((name, path))
}

/// Creates a [`VariableWithValue`] from a `StarlarkInfo`.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeaturesLib.java:198 CcToolchainFeaturesLib.variableWithValueFromStarlark
pub fn variable_with_value_from_starlark(
    variable_with_value_struct: &StarlarkInfo,
) -> Result<VariableWithValue, EvalException> {
    check_right_provider_type(variable_with_value_struct, "variable_with_value")?;
    let name = get_mandatory_string(variable_with_value_struct, "name")?;
    let value = get_mandatory_string(variable_with_value_struct, "value")?;
    let Some(name) = name.filter(|n| !n.is_empty()) else {
        return Err(info_error(
            variable_with_value_struct,
            "'name' parameter of variable_with_value must be a nonempty string.".to_string(),
        ));
    };
    let Some(value) = value.filter(|v| !v.is_empty()) else {
        return Err(info_error(
            variable_with_value_struct,
            "'value' parameter of variable_with_value must be a nonempty string.".to_string(),
        ));
    };
    Ok(VariableWithValue {
        variable: name,
        value,
    })
}

/// Creates an [`EnvEntry`] from a `StarlarkInfo`.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeaturesLib.java:220 CcToolchainFeaturesLib.envEntryFromStarlark
pub fn env_entry_from_starlark(env_entry_struct: &StarlarkInfo) -> Result<EnvEntry, EvalException> {
    check_right_provider_type(env_entry_struct, "env_entry")?;
    let key = get_mandatory_string(env_entry_struct, "key")?;
    let value = get_mandatory_string(env_entry_struct, "value")?;
    let Some(key) = key.filter(|k| !k.is_empty()) else {
        return Err(info_error(
            env_entry_struct,
            "'key' parameter of env_entry must be a nonempty string.".to_string(),
        ));
    };
    let Some(value) = value.filter(|v| !v.is_empty()) else {
        return Err(info_error(
            env_entry_struct,
            "'value' parameter of env_entry must be a nonempty string.".to_string(),
        ));
    };
    let expand_if_available = get_optional_string(env_entry_struct, "expand_if_available")?;
    let parser = StringValueParser::new(&value)?;
    Ok(EnvEntry {
        key,
        value_chunks: parser.get_chunks(),
        expand_if_all_available: match expand_if_available {
            None => StringSet::default(),
            Some(v) => StringSet::of([v]),
        },
    })
}

/// Creates a [`WithFeatureSet`] from a `StarlarkInfo`.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeaturesLib.java:241 CcToolchainFeaturesLib.withFeatureSetFromStarlark
pub fn with_feature_set_from_starlark(
    with_feature_set_struct: &StarlarkInfo,
) -> Result<WithFeatureSet, EvalException> {
    check_right_provider_type(with_feature_set_struct, "with_feature_set")?;
    let features =
        get_string_set_from_starlark_provider_field(with_feature_set_struct, "features")?;
    let not_features =
        get_string_set_from_starlark_provider_field(with_feature_set_struct, "not_features")?;
    Ok(WithFeatureSet {
        features,
        not_features,
    })
}

fn with_feature_sets_from(
    s: &StarlarkInfo,
    field: &str,
) -> Result<WithFeatureSetSet, EvalException> {
    let mut with_feature_set_builder = indexmap::IndexSet::new();
    for with_feature_set_struct in &get_starlark_provider_list_from_starlark_field(s, field)? {
        with_feature_set_builder.insert(with_feature_set_from_starlark(with_feature_set_struct)?);
    }
    Ok(WithFeatureSetSet(with_feature_set_builder))
}

/// Creates an [`EnvSet`] from a `StarlarkInfo`.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeaturesLib.java:253 CcToolchainFeaturesLib.envSetFromStarlark
pub fn env_set_from_starlark(env_set_struct: &StarlarkInfo) -> Result<EnvSet, EvalException> {
    check_right_provider_type(env_set_struct, "env_set")?;
    let actions = get_string_set_from_starlark_provider_field(env_set_struct, "actions")?;
    if actions.is_empty() {
        return Err(info_error(
            env_set_struct,
            "actions parameter of env_set must be a nonempty list.".to_string(),
        ));
    }
    let mut env_entry_builder = Vec::new();
    let env_entry_structs =
        get_starlark_provider_list_from_starlark_field(env_set_struct, "env_entries")?;
    for env_entry_struct in &env_entry_structs {
        env_entry_builder.push(env_entry_from_starlark(env_entry_struct)?);
    }

    let with_feature_sets = with_feature_sets_from(env_set_struct, "with_features")?;
    Ok(EnvSet {
        actions,
        env_entries: env_entry_builder,
        with_feature_sets,
    })
}

/// Creates a [`FlagGroup`] from a `StarlarkInfo`.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeaturesLib.java:277 CcToolchainFeaturesLib.flagGroupFromStarlark
pub fn flag_group_from_starlark(
    flag_group_struct: &StarlarkInfo,
) -> Result<FlagGroup, EvalException> {
    check_right_provider_type(flag_group_struct, "flag_group")?;

    let mut expandable_builder = Vec::new();
    let flags = get_string_list_from_starlark_provider_field(flag_group_struct, "flags")?;
    for flag in &flags {
        let parser = StringValueParser::new(flag)?;
        expandable_builder.push(Flag::create(parser.get_chunks()));
    }

    let flag_groups =
        get_starlark_provider_list_from_starlark_field(flag_group_struct, "flag_groups")?;
    for flag_group in &flag_groups {
        expandable_builder.push(crate::cc_toolchain_features::Expandable::FlagGroup(
            Box::new(flag_group_from_starlark(flag_group)?),
        ));
    }

    if !flag_groups.is_empty() && !flags.is_empty() {
        return Err(info_error(
            flag_group_struct,
            "flag_group must contain either a list of flags or a list of flag_groups.".to_string(),
        ));
    }

    if flag_groups.is_empty() && flags.is_empty() {
        return Err(info_error(
            flag_group_struct,
            "Both 'flags' and 'flag_groups' are empty.".to_string(),
        ));
    }

    let iterate_over = get_mandatory_string(flag_group_struct, "iterate_over")?;
    let expand_if_available = get_mandatory_string(flag_group_struct, "expand_if_available")?;
    let expand_if_not_available =
        get_mandatory_string(flag_group_struct, "expand_if_not_available")?;
    let expand_if_true = get_mandatory_string(flag_group_struct, "expand_if_true")?;
    let expand_if_false = get_mandatory_string(flag_group_struct, "expand_if_false")?;
    let expand_if_equal_struct = get_field_from_starlark_provider(
        flag_group_struct,
        "expand_if_equal",
        FieldClass::StarlarkInfo,
        true,
    )?;
    let expand_if_equal = match expand_if_equal_struct {
        Some(StarlarkInfoValue::Info(s)) => Some(variable_with_value_from_starlark(s)?),
        _ => None,
    };

    Ok(FlagGroup {
        expandables: expandable_builder,
        iterate_over_variable: iterate_over,
        expand_if_all_available: match expand_if_available {
            None => StringSet::default(),
            Some(v) => StringSet::of([v]),
        },
        expand_if_none_available: match expand_if_not_available {
            None => StringSet::default(),
            Some(v) => StringSet::of([v]),
        },
        expand_if_true,
        expand_if_false,
        expand_if_equal,
    })
}

/// Creates a [`FlagSet`] from a `StarlarkInfo`.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeaturesLib.java:332 CcToolchainFeaturesLib.flagSetFromStarlark
pub fn flag_set_from_starlark(
    flag_set_struct: &StarlarkInfo,
    action_name: Option<&str>,
) -> Result<FlagSet, EvalException> {
    check_right_provider_type(flag_set_struct, "flag_set")?;
    let mut actions = get_string_set_from_starlark_provider_field(flag_set_struct, "actions")?;
    // if we are creating a flag set for an action_config, we need to propagate the name of the
    // action to its flag_set.action_names
    if let Some(action_name) = action_name {
        if !actions.is_empty() {
            return Err(EvalException::new(
                ActionConfig::flag_set_with_action_error(action_name),
            ));
        }
        actions = StringSet::of([action_name]);
    }
    let mut flag_groups_builder = Vec::new();
    let flag_groups =
        get_starlark_provider_list_from_starlark_field(flag_set_struct, "flag_groups")?;
    for flag_group in &flag_groups {
        flag_groups_builder.push(flag_group_from_starlark(flag_group)?);
    }

    let with_feature_sets = with_feature_sets_from(flag_set_struct, "with_features")?;

    Ok(FlagSet {
        actions,
        expand_if_all_available: StringSet::default(),
        with_feature_sets,
        flag_groups: flag_groups_builder,
    })
}

/// Creates a [`Tool`] from a `StarlarkInfo`.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeaturesLib.java:364 CcToolchainFeaturesLib.toolFromStarlark
pub fn tool_from_starlark(tool_struct: &StarlarkInfo, exec_os: OS) -> Result<Tool, EvalException> {
    check_right_provider_type(tool_struct, "tool")?;

    let tool_path_string = get_optional_string(tool_struct, "path")?;
    let tool_artifact =
        match get_field_from_starlark_provider(tool_struct, "tool", FieldClass::Artifact, false)? {
            Some(StarlarkInfoValue::Artifact(a)) => Some(a.clone()),
            _ => None,
        };

    let tool_path: PathFragment;
    let tool_path_origin: PathOrigin;
    if let Some(tool_path_string) = tool_path_string {
        if tool_artifact.is_some() {
            return Err(info_error(
                tool_struct,
                "\"tool\" and \"path\" cannot be set at the same time.".to_string(),
            ));
        }

        tool_path = path_fragment_create_for_os(&tool_path_string, exec_os);
        if tool_path.is_empty() {
            return Err(info_error(
                tool_struct,
                "The 'path' field of tool must be a nonempty string.".to_string(),
            ));
        }

        if tool_path.is_absolute() {
            tool_path_origin = PathOrigin::FILESYSTEM_ROOT;
        } else {
            tool_path_origin = PathOrigin::CROSSTOOL_PACKAGE;
        }
    } else if let Some(tool_artifact) = tool_artifact {
        tool_path = tool_artifact.get_exec_path();
        tool_path_origin = PathOrigin::WORKSPACE_ROOT;
    } else {
        return Err(EvalException::new(
            "Exactly one of \"tool\" and \"path\" must be set.",
        ));
    }

    let with_feature_sets = with_feature_sets_from(tool_struct, "with_features")?;

    let execution_requirements =
        get_string_set_from_starlark_provider_field(tool_struct, "execution_requirements")?;
    Tool::new(
        tool_path,
        tool_path_origin,
        execution_requirements,
        with_feature_sets,
    )
}

/// Creates an [`ActionConfig`] from a `StarlarkInfo`.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeaturesLib.java:412 CcToolchainFeaturesLib.actionConfigFromStarlark
pub fn action_config_from_starlark(
    action_config_struct: &StarlarkInfo,
    exec_os: OS,
) -> Result<ActionConfig, EvalException> {
    check_right_provider_type(action_config_struct, "action_config")?;
    let action_name = get_mandatory_string(action_config_struct, "action_name")?;
    let Some(action_name) = action_name.filter(|a| !a.is_empty()) else {
        return Err(info_error(
            action_config_struct,
            "The 'action_name' field of action_config must be a nonempty string.".to_string(),
        ));
    };
    if !NAME_PATTERN.is_match(&action_name) {
        return Err(info_error(
            action_config_struct,
            format!(
                "An action_config's name must consist solely of lowercase ASCII letters, digits, \
                 '.', '_', '+', and '-', got '{action_name}'"
            ),
        ));
    }

    let enabled = get_mandatory_bool(action_config_struct, "enabled")?
        .unwrap_or_else(|| panic!("NullPointerException"));

    let mut tool_builder = Vec::new();
    let tool_structs =
        get_starlark_provider_list_from_starlark_field(action_config_struct, "tools")?;
    for tool_struct in &tool_structs {
        tool_builder.push(tool_from_starlark(tool_struct, exec_os)?);
    }

    let mut flag_set_builder = Vec::new();
    let flag_sets =
        get_starlark_provider_list_from_starlark_field(action_config_struct, "flag_sets")?;
    for flag_set in &flag_sets {
        flag_set_builder.push(flag_set_from_starlark(flag_set, Some(&action_name))?);
    }

    let implies = get_string_list_from_starlark_provider_field(action_config_struct, "implies")?;

    Ok(ActionConfig::new(
        action_name.clone(),
        action_name,
        tool_builder,
        flag_set_builder,
        enabled,
        implies,
    ))
}

// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeaturesLib.java:455 CcToolchainFeaturesLib.artifactNamePatternFromStarlark
pub fn artifact_name_pattern_from_starlark(
    artifact_name_pattern_struct: &StarlarkInfo,
    adder: &mut dyn FnMut(ArtifactCategory, &str, &str),
) -> Result<(), EvalException> {
    check_right_provider_type(artifact_name_pattern_struct, "artifact_name_pattern")?;
    let category_name = get_mandatory_string(artifact_name_pattern_struct, "category_name")?;
    let Some(category_name) = category_name.filter(|c| !c.is_empty()) else {
        return Err(info_error(
            artifact_name_pattern_struct,
            "The 'category_name' field of artifact_name_pattern must be a nonempty string."
                .to_string(),
        ));
    };
    let mut found_category = None;
    for artifact_category in ArtifactCategory::VALUES {
        if category_name == artifact_category.get_category_name() {
            found_category = Some(artifact_category);
        }
    }

    let Some(found_category) = found_category else {
        return Err(info_error(
            artifact_name_pattern_struct,
            format!("Artifact category {category_name} not recognized."),
        ));
    };

    let extension =
        get_mandatory_string(artifact_name_pattern_struct, "extension")?.unwrap_or_default();
    if !found_category
        .get_allowed_extensions()
        .contains(&extension.as_str())
    {
        return Err(info_error(
            artifact_name_pattern_struct,
            format!(
                "Unrecognized file extension '{}', allowed extensions are {}, please check \
                 artifact_name_pattern configuration for {} in your rule.",
                extension,
                join_english_list_single_quoted(found_category.get_allowed_extensions()),
                found_category.get_category_name()
            ),
        ));
    }

    let prefix = get_mandatory_string(artifact_name_pattern_struct, "prefix")?.unwrap_or_default();
    adder(found_category, &prefix, &extension);
    Ok(())
}

/// Java: `StringUtil.joinEnglishListSingleQuoted`.
// Java: com/google/devtools/build/lib/util/StringUtil.java:86 StringUtil.joinEnglishListSingleQuoted
fn join_english_list_single_quoted(choices: &[&str]) -> String {
    // Java: com/google/devtools/build/lib/util/StringUtil.java:54 StringUtil.joinEnglishList
    let (last_separator, quote, oxford_comma) = ("or", "'", true);
    let mut buf = String::new();
    let mut num_choices_seen = 0;
    let mut iter = choices.iter().peekable();
    while let Some(choice) = iter.next() {
        let has_next = iter.peek().is_some();
        if !buf.is_empty() {
            if has_next || (oxford_comma && num_choices_seen >= 2) {
                buf.push(',');
            }
            if !has_next {
                buf.push(' ');
                buf.push_str(last_separator);
            }
            buf.push(' ');
        }
        buf.push_str(quote);
        buf.push_str(choice);
        buf.push_str(quote);
        num_choices_seen += 1;
    }
    if buf.is_empty() {
        "nothing".to_string()
    } else {
        buf
    }
}
