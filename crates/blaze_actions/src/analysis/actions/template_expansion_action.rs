//! Ports of `lib/analysis/actions/TemplateExpansionAction.java`, the analysis-time parts of
//! `LocalTemplateExpansionStrategy.java` and `TemplateExpansionContext.java`,
//! `TemplateExpansionException.java`, `AqueryUtils.getTemplateContent` and
//! `StringUtilities.replaceAllLiteral`.

use std::any::Any;
use std::io;

use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_util::Fingerprint;
use indexmap::IndexMap;

use crate::actions::action::{
    AbstractAction, Action, ComputeKeyError, InputMetadataProvider, java_list_to_string,
};
use crate::actions::action_key_context::ActionKeyContext;
use crate::actions::action_owner::ActionOwner;
use crate::actions::artifact::Artifact;

use super::substitution::Substitution;
use super::template::Template;

// Java: com/google/devtools/build/lib/analysis/actions/TemplateExpansionAction.java:50 TemplateExpansionAction.GUID
const GUID: &str = "786c1fe0-dca8-407a-b108-e1ecd6d1bc7f";

/// The checked exceptions of `getFileContents` / `getExpandedTemplateUnsafe`: `IOException`,
/// `EvalException`.
#[derive(Debug, thiserror::Error)]
pub enum TemplateExpansionError {
    #[error("{0}")]
    Io(io::Error),
    /// `EvalException` (its message).
    #[error("{0}")]
    Eval(String),
}

/// Thrown during template expansion.
// Java: com/google/devtools/build/lib/analysis/actions/TemplateExpansionException.java:20 TemplateExpansionException
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{user_visible_error_message}")]
pub struct TemplateExpansionException {
    /// Error string that will be displayed to the user.
    pub user_visible_error_message: String,
    /// The original bzl evaluation failure (its message), for debugging only.
    pub cause: String,
}

impl TemplateExpansionException {
    // Java: com/google/devtools/build/lib/analysis/actions/TemplateExpansionException.java:30 TemplateExpansionException.<init>
    pub fn new(
        user_visible_error_message: impl Into<String>,
        cause: impl Into<String>,
    ) -> TemplateExpansionException {
        TemplateExpansionException {
            user_visible_error_message: user_visible_error_message.into(),
            cause: cause.into(),
        }
    }
}

/// Placeholder for metadata associated with a template.
// Java: com/google/devtools/build/lib/analysis/actions/TemplateExpansionContext.java:32 TemplateExpansionContext.TemplateMetadata
#[derive(Clone, Debug)]
pub struct TemplateExpansionContextTemplateMetadata {
    pub template: Template,
    pub primary_output: Artifact,
    pub substitutions: Vec<Substitution>,
    pub make_executable: bool,
}

/// Replace all occurrences of `literal` in `input` with `replacement`. If `literal` is empty,
/// returns `input` unchanged.
// Java: com/google/devtools/build/lib/util/StringUtilities.java:70 StringUtilities.replaceAllLiteral
// PORT: Java indexes chars of internal (Latin-1-of-UTF-8) strings, i.e. bytes; `str::find` on the
// UTF-8 form finds the same occurrences.
pub fn replace_all_literal(input: &str, literal: &str, replacement: &str) -> String {
    let literal_length = literal.len();
    if literal_length == 0 {
        return input.to_string();
    }
    let mut result = String::with_capacity(input.len() + replacement.len());
    let mut start = 0;
    while let Some(rel) = input[start..].find(literal) {
        let index = start + rel;
        result.push_str(&input[start..index]);
        result.push_str(replacement);
        start = index + literal_length;
    }
    result.push_str(&input[start..]);
    result
}

/// Get the result of the template expansion prior to executing the action.
// Java: com/google/devtools/build/lib/analysis/actions/LocalTemplateExpansionStrategy.java:75 LocalTemplateExpansionStrategy.getExpandedTemplateUnsafe
// PORT: the `ArtifactPathResolver` is always `IDENTITY` here (see `Template::get_content`).
pub fn get_expanded_template_unsafe(
    template: &Template,
    substitutions: &[Substitution],
) -> Result<String, TemplateExpansionError> {
    let mut template_string = template.get_content().map_err(TemplateExpansionError::Io)?;
    for entry in substitutions {
        let value = entry.get_value().map_err(TemplateExpansionError::Eval)?;
        template_string = replace_all_literal(&template_string, entry.get_key(), &value);
    }
    Ok(template_string)
}

/// Action to expand a template and write the expanded content to a file.
// Java: com/google/devtools/build/lib/analysis/actions/TemplateExpansionAction.java:48 TemplateExpansionAction
#[derive(Debug)]
pub struct TemplateExpansionAction {
    base: AbstractAction,
    template: Template,
    substitutions: Vec<Substitution>,
    make_executable: bool,
}

impl TemplateExpansionAction {
    // Java: com/google/devtools/build/lib/analysis/actions/TemplateExpansionAction.java:67 TemplateExpansionAction.<init>
    fn new_internal(
        owner: ActionOwner,
        inputs: NestedSet<Artifact>,
        primary_output: Artifact,
        template: Template,
        substitutions: Vec<Substitution>,
        make_executable: bool,
    ) -> TemplateExpansionAction {
        TemplateExpansionAction {
            base: AbstractAction::new(owner, inputs, [primary_output]),
            template,
            substitutions,
            make_executable,
        }
    }

    /// Creates a new TemplateExpansionAction instance for an artifact template.
    ///
    /// `template_artifact` is read as the text template file; the substitutions are applied in
    /// order.
    // Java: com/google/devtools/build/lib/analysis/actions/TemplateExpansionAction.java:92 TemplateExpansionAction.<init>
    pub fn for_template_artifact(
        owner: ActionOwner,
        template_artifact: Artifact,
        output: Artifact,
        substitutions: Vec<Substitution>,
        make_executable: bool,
    ) -> TemplateExpansionAction {
        TemplateExpansionAction::new_internal(
            owner,
            NestedSetBuilder::create(Order::STABLE_ORDER, [template_artifact.clone()]),
            output,
            Template::for_artifact(template_artifact),
            substitutions,
            make_executable,
        )
    }

    /// Creates a new TemplateExpansionAction instance without inputs.
    // Java: com/google/devtools/build/lib/analysis/actions/TemplateExpansionAction.java:117 TemplateExpansionAction.<init>
    pub fn new(
        owner: ActionOwner,
        output: Artifact,
        template: Template,
        substitutions: Vec<Substitution>,
        make_executable: bool,
    ) -> TemplateExpansionAction {
        TemplateExpansionAction::new_internal(
            owner,
            NestedSetBuilder::empty_set(Order::STABLE_ORDER),
            output,
            template,
            substitutions,
            make_executable,
        )
    }

    /// The metadata `execute` hands to the `TemplateExpansionContext`.
    // Java: com/google/devtools/build/lib/analysis/actions/TemplateExpansionAction.java:158 TemplateExpansionAction.execute
    // PORT: `execute` itself is execution (wave W6); this builds its `TemplateMetadata`.
    pub fn get_template_metadata(&self) -> TemplateExpansionContextTemplateMetadata {
        TemplateExpansionContextTemplateMetadata {
            template: self.template.clone(),
            primary_output: self.get_primary_output(),
            substitutions: self.substitutions.clone(),
            make_executable: self.make_executable,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/actions/TemplateExpansionAction.java:172 TemplateExpansionAction.getFileContents
    pub fn get_file_contents(&self) -> Result<String, TemplateExpansionError> {
        get_expanded_template_unsafe(&self.template, &self.substitutions)
    }

    // Java: com/google/devtools/build/lib/analysis/actions/TemplateExpansionAction.java:178 TemplateExpansionAction.getStarlarkContent
    pub fn get_starlark_content(&self) -> Result<String, TemplateExpansionError> {
        self.get_file_contents()
    }

    // Java: com/google/devtools/build/lib/analysis/actions/TemplateExpansionAction.java:215 TemplateExpansionAction.getSubstitutions
    pub fn get_substitutions(&self) -> &[Substitution] {
        &self.substitutions
    }

    // Java: com/google/devtools/build/lib/analysis/actions/TemplateExpansionAction.java:219 TemplateExpansionAction.getTemplate
    pub fn get_template(&self) -> &Template {
        &self.template
    }

    // Java: com/google/devtools/build/lib/analysis/actions/TemplateExpansionAction.java:223 TemplateExpansionAction.makeExecutable
    pub fn make_executable(&self) -> bool {
        self.make_executable
    }

    /// `Err` is the `EvalException` message.
    // Java: com/google/devtools/build/lib/analysis/actions/TemplateExpansionAction.java:228 TemplateExpansionAction.getStarlarkSubstitutions
    pub fn get_starlark_substitutions(&self) -> Result<IndexMap<String, String>, String> {
        let mut builder = IndexMap::new();
        for entry in &self.substitutions {
            builder.insert(entry.get_key().to_string(), entry.get_value()?);
        }
        Ok(builder)
    }
}

/// The template content aquery reports for a template expansion.
// Java: com/google/devtools/build/lib/query2/aquery/AqueryUtils.java:111 AqueryUtils.getTemplateContent
pub fn get_template_content(action: &TemplateExpansionAction) -> io::Result<String> {
    // If the template artifact is a DerivedArtifact, it is only available during the execution
    // phase. It's therefore not possible to read its content from the FileSystem at this moment.
    if action
        .get_template()
        .get_template_artifact()
        .is_some_and(|a| a.is_derived_artifact_instance())
    {
        return Ok(action.get_template().to_string());
    }
    action.get_template().get_content()
}

impl Action for TemplateExpansionAction {
    fn abstract_action(&self) -> &AbstractAction {
        &self.base
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    // Java: com/google/devtools/build/lib/analysis/actions/TemplateExpansionAction.java:206 TemplateExpansionAction.getMnemonic
    fn get_mnemonic(&self) -> String {
        "TemplateExpand".to_string()
    }

    // Java: com/google/devtools/build/lib/analysis/actions/TemplateExpansionAction.java:183 TemplateExpansionAction.computeKey
    fn compute_key(
        &self,
        _action_key_context: &ActionKeyContext,
        _input_metadata_provider: Option<&dyn InputMetadataProvider>,
        fp: &mut Fingerprint,
    ) -> Result<(), ComputeKeyError> {
        fp.add_string(GUID);
        fp.add_string(if self.make_executable {
            "true"
        } else {
            "false"
        });
        // Invariant: the template text and the substitutions are internal strings (see
        // `fingerprint_util`).
        fp.add_string(&self.template.get_key());
        fp.add_int(self.substitutions.len() as i32);
        for entry in &self.substitutions {
            fp.add_string(entry.get_key());
            let value = entry.get_value().map_err(ComputeKeyError::Eval)?;
            fp.add_string(&value);
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/analysis/actions/TemplateExpansionAction.java:199 TemplateExpansionAction.describeKey
    fn describe_key(&self) -> Option<String> {
        Some(format!(
            "GUID: {}\nmakeExecutable: {}\ntemplate: {}\nsubstitutions: {}\n",
            GUID,
            self.make_executable,
            self.template.get_key(),
            java_list_to_string(self.substitutions.iter())
        ))
    }

    // Java: com/google/devtools/build/lib/analysis/actions/TemplateExpansionAction.java:211 TemplateExpansionAction.getRawProgressMessage
    fn get_raw_progress_message(&self) -> Option<String> {
        let outputs = self.get_outputs();
        blaze_util::check_argument!(
            outputs.len() == 1,
            "expected one element but was: {:?}",
            outputs
        );
        Some(format!("Expanding template {}", outputs[0].pretty_print()))
    }

    // Java: com/google/devtools/build/lib/query2/aquery/AqueryUtils.java:111 AqueryUtils.getTemplateContent
    fn aquery_template_expansion(&self) -> Option<Result<(String, Vec<(String, String)>), String>> {
        let content = match get_template_content(self) {
            Ok(c) => c,
            Err(e) => return Some(Err(e.to_string())),
        };
        let mut substitutions = Vec::with_capacity(self.substitutions.len());
        for s in &self.substitutions {
            match s.get_value() {
                Ok(v) => substitutions.push((s.get_key().to_string(), v)),
                Err(e) => return Some(Err(e)),
            }
        }
        Some(Ok((content, substitutions)))
    }
}
