//! Event indicating that a repository rule was executed, together with the return value of the
//! rule.
//!
//! Java: com/google/devtools/build/lib/bazel/repository/RepositoryResolvedEvent.java.

use blaze_bzlmod_model::AttributeValue;
use indexmap::IndexMap;
use starlark::environment::Module;

use super::repo_definition::RepoDefinition;
use crate::attr_values::attr_value_to_attribute_value;
use crate::attr_values::attribute_value_to_starlark;

/// Java `Starlark.repr(value)` of a stored attribute value.
fn starlark_repr(value: &AttributeValue) -> String {
    Module::with_temp_heap(|module| attribute_value_to_starlark(value, module.heap()).to_repr())
}

/// The outcome of comparing one attribute (Java's `Optional<Object>` map values): `None` means
/// dropped, `Some(v)` modified to the non-default value `v`.
type Modification = Option<AttributeValue>;

/// Event indicating that a repository rule was executed, together with the return value of the
/// rule.
// Java: com/google/devtools/build/lib/bazel/repository/RepositoryResolvedEvent.java:28 RepositoryResolvedEvent
#[derive(Clone, Debug)]
pub struct RepositoryResolvedEvent {
    information_returned: bool,
    message: String,
}

impl RepositoryResolvedEvent {
    /// PORT: Java compares Starlark/Java objects with `equals` (a native default value against
    /// the Starlark values in the repo definition and the rule's return value); Blaze compares
    /// their stored forms ([`AttributeValue`]), with the default converted by
    /// `Attribute.valueToStarlark`. The two agree for every attribute type a repo rule can have.
    // Java: com/google/devtools/build/lib/bazel/repository/RepositoryResolvedEvent.java:33 RepositoryResolvedEvent.<init>
    pub fn new(
        repo_definition: &RepoDefinition,
        result: &IndexMap<String, AttributeValue>,
    ) -> Self {
        if result.is_empty() {
            // Repo claims to be already reproducible, so wants to be called as is.
            return RepositoryResolvedEvent {
                information_returned: false,
                message: format!("Repo '{}' finished fetching.", repo_definition.name),
            };
        }
        // Repo claims that the returned (probably changed) arguments are a reproducible version
        // of itself. Diff them and report the changes, if any.
        let mut modified_attributes: Vec<(String, Modification)> = Vec::new();
        for name in repo_definition.get_field_names() {
            // The "name" attribute is confusing as the value specified by the user is transformed
            // to the canonical name for repository_ctx.attr.name. Since the name should never
            // affect reproducibility, ignore it.
            if name == "name" {
                continue;
            }
            // Filter out implicit attributes, which can't be modified by the user.
            if name.starts_with('_') {
                continue;
            }
            let repo_rule = &repo_definition.repo_rule;
            let attribute = &repo_rule.attributes[repo_rule.attribute_indices[&name]];
            // Label attributes report a default of null rather than None.
            let default_value =
                attr_value_to_attribute_value(attribute.get_default_value_unchecked());
            // Java: repoDefinition.getValue(name) for a declared attribute.
            let current_value = match repo_definition.attr_values.get(&name) {
                Some(v) => v.clone(),
                None => default_value.clone(),
            };
            let new_value = result.get(&name).unwrap_or(&default_value);
            if *new_value == current_value {
                continue;
            }
            // Distinguish between "dropped" and non-trivially "modified" attributes.
            let modification = if *new_value == default_value {
                None
            } else {
                Some(new_value.clone())
            };
            modified_attributes.push((name, modification));
        }
        if modified_attributes.is_empty() {
            return RepositoryResolvedEvent {
                information_returned: false,
                message: format!("Repo '{}' finished fetching.", repo_definition.name),
            };
        }
        let modified_to_non_default: Vec<(String, AttributeValue)> = modified_attributes
            .iter()
            .filter_map(|(k, v)| v.as_ref().map(|v| (k.clone(), v.clone())))
            .collect();
        let dropped: Vec<&str> = modified_attributes
            .iter()
            .filter(|(_, v)| v.is_none())
            .map(|(k, _)| k.as_str())
            .collect();
        // Java: Starlark.repr of a (non-Starlark) Java Set is its toString(): `[a, b]`.
        let dropped_repr = format!("[{}]", dropped.join(", "));
        let message = if modified_to_non_default.is_empty() {
            format!(
                "Repo '{}' indicated that a canonical reproducible form can be obtained by \
                 dropping arguments {}",
                repo_definition.name, dropped_repr
            )
        } else if dropped.is_empty() {
            format!(
                "Repo '{}' indicated that a canonical reproducible form can be obtained by \
                 modifying arguments {}",
                repo_definition.name,
                Self::represent_modifications(&modified_to_non_default)
            )
        } else {
            format!(
                "Repo '{}' indicated that a canonical reproducible form can be obtained by \
                 modifying arguments {} and dropping {}",
                repo_definition.name,
                Self::represent_modifications(&modified_to_non_default),
                dropped_repr
            )
        };
        RepositoryResolvedEvent {
            information_returned: true,
            message,
        }
    }

    /// True, if the return value of the repository rule contained new information with respect
    /// to the way it was called.
    // Java: com/google/devtools/build/lib/bazel/repository/RepositoryResolvedEvent.java:114 RepositoryResolvedEvent.isNewInformationReturned
    pub fn is_new_information_returned(&self) -> bool {
        self.information_returned
    }

    /// Message describing the event
    // Java: com/google/devtools/build/lib/bazel/repository/RepositoryResolvedEvent.java:119 RepositoryResolvedEvent.getMessage
    pub fn get_message(&self) -> &str {
        &self.message
    }

    /// Returns an unstructured message explaining the origin of this rule.
    // Java: com/google/devtools/build/lib/bazel/repository/RepositoryResolvedEvent.java:124 RepositoryResolvedEvent.getRuleDefinitionInformation
    pub fn get_rule_definition_information(repo_definition: &RepoDefinition) -> String {
        // We used to output a call stack for repos defined in WORKSPACE, but in Bzlmod we always
        // get an empty stack -- for repos backing modules there's no call stack; for
        // extension-generated repos, the call stack is lost during the roundtrip to the lockfile.
        // TODO: store the call stack in the lockfile and output it here?
        let id = &repo_definition.repo_rule.id;
        format!(
            "Repo {} defined by rule {} in {}",
            repo_definition.name,
            id.rule_name(),
            id.bzl_file_label()
                .expect("NullPointerException: RepoRuleId.bzlFileLabel is null")
                .get_unambiguous_canonical_form()
        )
    }

    // Java: com/google/devtools/build/lib/bazel/repository/RepositoryResolvedEvent.java:136 RepositoryResolvedEvent.representModifications
    pub(crate) fn represent_modifications(changes: &[(String, AttributeValue)]) -> String {
        changes
            .iter()
            .map(|(k, v)| format!("{} = {}", k, starlark_repr(v)))
            .collect::<Vec<_>>()
            .join(", ")
    }
}
