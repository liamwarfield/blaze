//! Utilities related to processing attributes in external deps contexts, and
//! `RepoRule.instantiate`.
//!
//! Java: com/google/devtools/build/lib/bazel/repository/AttributeUtils.java and
//! com/google/devtools/build/lib/bazel/repository/RepoRule.java (`instantiate`).

use blaze_bzlmod_model::AttributeValue;
use blaze_bzlmod_model::AttributeValues;
use blaze_bzlmod_model::ExternalDepsCode;
use blaze_bzlmod_model::ExternalDepsException;
use blaze_bzlmod_model::RepoSpec;
use blaze_cmdline::Label;
use blaze_events::Event;
use blaze_events::ExtendedEventHandler;
use blaze_options::spelling::SpellChecker;
use blaze_packages::attribute::Attribute;
use blaze_packages::types::label_converter::LabelConverter;
use blaze_starlark::EvalError;
use blaze_starlark::error::EvalErrorKind;
use blaze_starlark::error::StackEntry;
use indexmap::IndexMap;
use starlark::values::Value;

use super::repo_rule::LEGACY_BUILTIN_ATTRIBUTES;
use super::repo_rule::RepoRule;
use crate::attr_values::attr_value_to_attribute_value;

/// Java `ExternalDepsException.withCallStackAndMessage(code, callStack, format, args)`: the
/// message is `EvalException.formatCallStack(callStack, message, ...)`.
///
/// PORT: move to blaze_bzlmod_model (`ExternalDepsException::with_call_stack_and_message`).
// Java: com/google/devtools/build/lib/bazel/bzlmod/ExternalDepsException.java:51 ExternalDepsException.withCallStackAndMessage
pub fn external_deps_exception_with_call_stack_and_message(
    code: ExternalDepsCode,
    call_stack: &[StackEntry],
    message: String,
) -> ExternalDepsException {
    // Java: net/starlark/java/eval/EvalException.java:200 EvalException.formatCallStack
    let formatted = EvalError {
        kind: EvalErrorKind::Eval,
        message,
        callstack: call_stack.to_vec(),
        include_stack: true,
    }
    .message_with_stack();
    ExternalDepsException::with_message(code, formatted)
}

/// The event location of a call stack entry (`Location.BUILTIN` for a builtin).
fn event_location(entry: &StackEntry) -> blaze_events::Location {
    match &entry.location {
        Some(location) => blaze_sky_loading::starlark_util::event_location(location),
        None => blaze_events::Location::builtin().clone(),
    }
}

// Java: com/google/devtools/build/lib/bazel/repository/AttributeUtils.java:139 AttributeUtils.nonVisibleLabelsIn
fn non_visible_labels_in(native_attr_value: &AttributeValue) -> Option<&Label> {
    match native_attr_value {
        AttributeValue::Label(label) if !label.get_repository().is_visible() => Some(label),
        AttributeValue::List(list) => {
            for item in list {
                let non_visible_label = non_visible_labels_in(item);
                if non_visible_label.is_some() {
                    return non_visible_label;
                }
            }
            None
        }
        AttributeValue::Dict(map) => {
            for key_or_value in map.iter().map(|(k, _)| k).chain(map.iter().map(|(_, v)| v)) {
                let non_visible_label = non_visible_labels_in(key_or_value);
                if non_visible_label.is_some() {
                    return non_visible_label;
                }
            }
            None
        }
        _ => None,
    }
}

/// Type-checks the given attribute values against a defined attribute schema, potentially
/// converting the values wherever necessary.
///
/// - `attrs` with `attr_indices` defines the attribute schema.
/// - `kwargs`: the supplied attribute values (keyed by the attribute names), in order.
/// - `where_`: a context string used in error messages to denote where this typechecking is
///   happening.
/// - `repo_mapping_where`: a context string used in error messages about invalid apparent repo
///   names, to denote where this repo mapping is anchored.
///
/// Returns the type-checked and converted values (Java `Attribute.valueToStarlark` of the native
/// values, as stored data), in the same order as `attrs`.
// Java: com/google/devtools/build/lib/bazel/repository/AttributeUtils.java:51 AttributeUtils.typeCheckAttrValues
pub fn type_check_attr_values<'v>(
    attrs: &[Attribute],
    attr_indices: &IndexMap<String, usize>,
    kwargs: &[(String, Value<'v>)],
    label_converter: &LabelConverter,
    error_code: ExternalDepsCode,
    call_stack: &[StackEntry],
    where_: &str,
    repo_mapping_where: &str,
) -> Result<Vec<AttributeValue>, ExternalDepsException> {
    let mut attr_values: Vec<Option<AttributeValue>> = vec![None; attrs.len()];
    for (attr_name, attr_value) in kwargs {
        if attr_value.is_none() {
            continue;
        }
        let Some(&attr_index) = attr_indices.get(attr_name) else {
            return Err(external_deps_exception_with_call_stack_and_message(
                error_code,
                call_stack,
                format!(
                    "in {}, unknown attribute '{}' provided{}",
                    where_,
                    attr_name,
                    SpellChecker::did_you_mean(attr_name, attr_indices.keys())
                ),
            ));
        };
        let attr = &attrs[attr_index];
        let what = format!("attribute '{}'", attr.get_public_name());
        let native_value = match attr.get_type().convert(
            *attr_value,
            Some(&what as &dyn std::fmt::Display),
            Some(label_converter),
        ) {
            Ok(v) => v,
            Err(e) => {
                return Err(external_deps_exception_with_call_stack_and_message(
                    error_code,
                    call_stack,
                    format!("in {}, {}", where_, e),
                ));
            }
        };

        // Check that the value is actually allowed.
        if attr.check_allowed_values() {
            let allowed_values = attr
                .get_allowed_values()
                .expect("NullPointerException: allowed values");
            if !allowed_values.apply(&native_value) {
                return Err(external_deps_exception_with_call_stack_and_message(
                    error_code,
                    call_stack,
                    format!(
                        "in {}, the value for attribute '{}' {}",
                        where_,
                        attr.get_public_name(),
                        allowed_values.get_error_reason(&native_value)
                    ),
                ));
            }
        }

        attr_values[attr_index] = Some(attr_value_to_attribute_value(Some(&native_value)));
    }

    // Check that all mandatory attributes have been specified, and fill in default values.
    // Along the way, verify that labels in the attribute values refer to visible repos only.
    let mut result = Vec::with_capacity(attr_values.len());
    for (i, attr_value) in attr_values.into_iter().enumerate() {
        let attr = &attrs[i];
        if attr.is_mandatory() && attr_value.is_none() {
            return Err(external_deps_exception_with_call_stack_and_message(
                error_code,
                call_stack,
                format!(
                    "in {}, mandatory attribute '{}' isn't being specified",
                    where_,
                    attr.get_public_name()
                ),
            ));
        }
        let attr_value = match attr_value {
            Some(v) => v,
            None => attr_value_to_attribute_value(attr.get_default_value_unchecked()),
        };
        if let Some(first_non_visible_label) = non_visible_labels_in(&attr_value) {
            return Err(external_deps_exception_with_call_stack_and_message(
                error_code,
                call_stack,
                format!(
                    "in {}, no repository visible as '@{}' {}, but referenced by label \
                     '@{}//{}:{}' in attribute '{}'",
                    where_,
                    first_non_visible_label.get_repository().get_name(),
                    repo_mapping_where,
                    first_non_visible_label.get_repository().get_name(),
                    first_non_visible_label.get_package_fragment(),
                    first_non_visible_label.get_name(),
                    attr.get_public_name()
                ),
            ));
        }
        result.push(attr_value);
    }
    Ok(result)
}

impl RepoRule {
    /// Type-checks `kwargs` (a call of this repo rule) and returns the repo spec of the call.
    /// Errors are also reported to `event_handler`.
    ///
    /// Stored attribute values (a `RepoSpec` from a registry or the lockfile) are passed as
    /// Starlark values made with `crate::attr_values::attribute_value_to_starlark`.
    // Java: com/google/devtools/build/lib/bazel/repository/RepoRule.java:87 RepoRule.instantiate
    pub fn instantiate<'v>(
        &self,
        kwargs: &[(String, Value<'v>)],
        call_stack: &[StackEntry],
        label_converter: &LabelConverter,
        event_handler: &dyn ExtendedEventHandler,
        repo_mapping_where: &str,
    ) -> Result<RepoSpec, ExternalDepsException> {
        let result = (|| -> Result<RepoSpec, ExternalDepsException> {
            let maybe_name = match kwargs
                .iter()
                .find(|(k, _)| k == "name")
                .and_then(|(_, v)| v.unpack_str())
            {
                Some(name) => format!(" with name '{}'", name),
                None => String::new(),
            };
            let filtered_kwargs: Vec<(String, Value<'v>)> = kwargs
                .iter()
                .filter(|(k, _)| !LEGACY_BUILTIN_ATTRIBUTES.contains(&k.as_str()))
                .cloned()
                .collect();
            let attr_values = type_check_attr_values(
                &self.attributes,
                &self.attribute_indices,
                &filtered_kwargs,
                label_converter,
                ExternalDepsCode::ExtensionEvalError,
                call_stack,
                &format!("call to '{}' repo rule{}", self.id.rule_name(), maybe_name),
                repo_mapping_where,
            )?;
            let mut attr_dict: Vec<(String, AttributeValue)> = Vec::new();
            for (key, value) in kwargs {
                // Only store explicitly-specified attributes.
                if !LEGACY_BUILTIN_ATTRIBUTES.contains(&key.as_str()) && !value.is_none() {
                    attr_dict.push((
                        key.clone(),
                        attr_values[self.attribute_indices[key.as_str()]].clone(),
                    ));
                }
            }
            Ok(RepoSpec::new(
                self.id.clone(),
                AttributeValues::create(attr_dict),
            ))
        })();
        if let Err(e) = &result {
            let last = call_stack.last().expect("NoSuchElementException");
            event_handler.handle(&Event::error_at(
                Some(event_location(last)),
                e.get_message(),
            ));
        }
        result
    }
}
