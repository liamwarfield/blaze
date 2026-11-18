//! Port of `actions/ActionConflictException.java` and of the identity-keyed map of bad actions
//! (`Map<ActionAnalysisMetadata, ActionConflictException>`) the conflict code passes around.
//!
//! PORT: the Java class lives in `lib/actions` (`blaze_actions`); it is kept here, in the lowest
//! crate of its callers (`rule::actions_util::verify_generating_action_keys` for a conflict
//! inside one target; `blaze_sky_top`'s `ArtifactConflictFinder` for conflicts between targets,
//! which re-exports it), until that crate takes it.

use std::fmt;
use std::sync::Arc;

use blaze_actions::actions::runfiles_tree_action::RunfilesTreeAction;
use blaze_actions::analysis::actions::binary_file_write_action::BinaryFileWriteAction;
use blaze_actions::analysis::actions::build_info_file_write_action::BuildInfoFileWriteAction;
use blaze_actions::analysis::actions::file_write_action::FileWriteAction;
use blaze_actions::analysis::actions::lazy_write_nested_set_of_tuple_action::LazyWriteNestedSetOfTupleAction;
use blaze_actions::analysis::actions::lazy_write_paths_file_action::LazyWritePathsFileAction;
use blaze_actions::analysis::actions::parameter_file_write_action::ParameterFileWriteAction;
use blaze_actions::analysis::actions::spawn_action::{SpawnAction, SpawnActionClass};
use blaze_actions::analysis::actions::symlink_action::SymlinkAction;
use blaze_actions::analysis::actions::symlink_tree_action::SymlinkTreeAction;
use blaze_actions::analysis::actions::template_expansion_action::TemplateExpansionAction;
use blaze_actions::analysis::repo_mapping_manifest_action::RepoMappingManifestAction;
use blaze_actions::analysis::source_manifest_action::SourceManifestAction;
use blaze_actions::analysis::starlark::unresolved_symlink_action::UnresolvedSymlinkAction;
use blaze_actions::{ActionKeyContext, ActionRef, Artifact};
use blaze_cmdline::Label;
use blaze_events::{Event, EventHandler};
use blaze_protos::failure_details::{self, FailureDetail, failure_detail::Category};
use indexmap::IndexMap;

// Java: com/google/devtools/build/lib/actions/ActionConflictException.java:46 ActionConflictException.MAX_DIFF_ARTIFACTS_TO_REPORT
const MAX_DIFF_ARTIFACTS_TO_REPORT: usize = 5;

/// This exception is thrown when a conflict between actions is detected. It contains information
/// about the artifact for which the conflict is found, and data about the two conflicting actions
/// and their owners.
///
/// PORT: `WithAspectKeyInfo` (the aspect key of a Skymeld conflict) is not ported: aspects and
/// Skymeld are not.
// Java: com/google/devtools/build/lib/actions/ActionConflictException.java:41 ActionConflictException
#[derive(Clone)]
pub struct ActionConflictException {
    message: String,
    artifact: Artifact,
    attempted_action: ActionRef,
    is_prefix_conflict: bool,
}

impl ActionConflictException {
    // Java: com/google/devtools/build/lib/actions/ActionConflictException.java:48 ActionConflictException.create
    pub fn create(
        action_key_context: &ActionKeyContext,
        artifact: &Artifact,
        previous_action: &ActionRef,
        attempted_action: &ActionRef,
    ) -> ActionConflictException {
        ActionConflictException {
            message: create_detailed_message(
                artifact,
                action_key_context,
                attempted_action,
                previous_action,
            ),
            artifact: artifact.clone(),
            attempted_action: attempted_action.clone(),
            is_prefix_conflict: false,
        }
    }

    // Java: com/google/devtools/build/lib/actions/ActionConflictException.java:60 ActionConflictException.create
    pub fn create_with_message(
        artifact: &Artifact,
        attempted_action: &ActionRef,
        message: String,
        is_prefix_conflict: bool,
    ) -> ActionConflictException {
        ActionConflictException {
            message,
            artifact: artifact.clone(),
            attempted_action: attempted_action.clone(),
            is_prefix_conflict,
        }
    }

    /// Exception to indicate that one `Action` has an output artifact whose path is a prefix of
    /// an output of another action. Since the first path cannot be both a directory and a file,
    /// this would lead to an error if both actions were executed in the same build.
    // Java: com/google/devtools/build/lib/actions/ActionConflictException.java:73 ActionConflictException.createPrefix
    pub fn create_prefix(
        first_artifact: &Artifact,
        second_artifact: &Artifact,
        first_action: &ActionRef,
        second_action: &ActionRef,
    ) -> ActionConflictException {
        ActionConflictException {
            message: create_prefix_detailed_message(
                first_artifact,
                second_artifact,
                first_action.get_owner().get_label(),
                second_action.get_owner().get_label(),
            ),
            artifact: first_artifact.clone(),
            attempted_action: first_action.clone(),
            is_prefix_conflict: true,
        }
    }

    /// Java `getMessage()`.
    pub fn get_message(&self) -> &str {
        &self.message
    }

    // Java: com/google/devtools/build/lib/actions/ActionConflictException.java:105 ActionConflictException.getArtifact
    pub fn get_artifact(&self) -> &Artifact {
        &self.artifact
    }

    // Java: com/google/devtools/build/lib/actions/ActionConflictException.java:109 ActionConflictException.getAttemptedAction
    pub fn get_attempted_action(&self) -> &ActionRef {
        &self.attempted_action
    }

    // Java: com/google/devtools/build/lib/actions/ActionConflictException.java:135 ActionConflictException.reportTo
    pub fn report_to(&self, event_listener: &dyn EventHandler) {
        event_listener.handle(&Event::error(self.get_message()));
    }

    /// `getDetailedExitCode().getFailureDetail()`. The exit code of both codes is 1
    /// (failure_details.proto).
    // Java: com/google/devtools/build/lib/actions/ActionConflictException.java:140 ActionConflictException.getDetailedExitCode
    pub fn get_failure_detail(&self) -> FailureDetail {
        FailureDetail {
            message: self.get_message().to_owned(),
            category: Some(Category::Analysis(failure_details::Analysis {
                code: if self.is_prefix_conflict {
                    failure_details::analysis::Code::ArtifactPrefixConflict as i32
                } else {
                    failure_details::analysis::Code::ActionConflict as i32
                },
            })),
        }
    }
}

impl fmt::Debug for ActionConflictException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActionConflictException")
            .field("message", &self.message)
            .field("artifact", &self.artifact)
            .field("is_prefix_conflict", &self.is_prefix_conflict)
            .finish()
    }
}

impl fmt::Display for ActionConflictException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ActionConflictException {}

/// `Map<ActionAnalysisMetadata, ActionConflictException>`: the actions in conflict, keyed by
/// the action's identity (Java's actions do not override `equals`), in insertion order.
///
/// PORT: Java's maps here are a `ConcurrentHashMap` and `ImmutableMap` copies of it, iterated
/// in identity-hash order (not reproducible); insertion order is used instead.
#[derive(Clone, Default)]
pub struct ActionConflicts {
    conflicts: IndexMap<usize, (ActionRef, ActionConflictException)>,
}

impl ActionConflicts {
    pub fn new() -> Self {
        ActionConflicts::default()
    }

    fn identity(action: &ActionRef) -> usize {
        Arc::as_ptr(action) as *const () as usize
    }

    /// `Map.put`.
    pub fn put(&mut self, action: &ActionRef, exception: ActionConflictException) {
        self.conflicts
            .insert(Self::identity(action), (action.clone(), exception));
    }

    /// `Map.putAll`.
    pub fn put_all(&mut self, other: &ActionConflicts) {
        for (action, exception) in other.entries() {
            self.put(action, exception.clone());
        }
    }

    /// `Map.containsKey`.
    pub fn contains_key(&self, action: &ActionRef) -> bool {
        self.conflicts.contains_key(&Self::identity(action))
    }

    /// `Map.get`.
    pub fn get(&self, action: &ActionRef) -> Option<&ActionConflictException> {
        self.conflicts.get(&Self::identity(action)).map(|(_, e)| e)
    }

    /// `Map.entrySet()`.
    pub fn entries(&self) -> impl Iterator<Item = (&ActionRef, &ActionConflictException)> {
        self.conflicts.values().map(|(a, e)| (a, e))
    }

    pub fn is_empty(&self) -> bool {
        self.conflicts.is_empty()
    }

    pub fn len(&self) -> usize {
        self.conflicts.len()
    }
}

impl fmt::Debug for ActionConflicts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.conflicts.values().map(|(_, e)| e))
            .finish()
    }
}

// Java: com/google/devtools/build/lib/actions/ActionConflictException.java:113 ActionConflictException.createDetailedMessage
fn create_detailed_message(
    artifact: &Artifact,
    action_key_context: &ActionKeyContext,
    a: &ActionRef,
    b: &ActionRef,
) -> String {
    format!(
        "file '{}' is generated by these conflicting actions:\n{}",
        artifact.pretty_print(),
        debug_suffix(action_key_context, a, b)
    )
}

// Java: com/google/devtools/build/lib/actions/ActionConflictException.java:125 ActionConflictException.createPrefixDetailedMessage
fn create_prefix_detailed_message(
    first_artifact: &Artifact,
    second_artifact: &Artifact,
    first_owner: Option<&Label>,
    second_owner: Option<&Label>,
) -> String {
    format!(
        "One of the output paths '{}' (belonging to {}) and '{}' (belonging to {}) is a prefix of \
         the other. These actions cannot be simultaneously present; please rename one of the \
         output files or build just one of them",
        first_artifact.get_exec_path(),
        java_string_value_of_label(first_owner),
        second_artifact.get_exec_path(),
        java_string_value_of_label(second_owner)
    )
}

/// `String.valueOf(label)` (a `%s` argument): `null` for a null label.
fn java_string_value_of_label(label: Option<&Label>) -> String {
    match label {
        Some(label) => label.to_string(),
        None => "null".to_owned(),
    }
}

/// `Label.print(label)`.
// Java: com/google/devtools/build/lib/cmdline/Label.java:691 Label.print
fn label_print(label: Option<&Label>) -> String {
    match label {
        None => "(unknown)".to_owned(),
        Some(label) => label.to_string(),
    }
}

// Java: com/google/devtools/build/lib/actions/ActionConflictException.java:151 ActionConflictException.addStringDetail
fn add_string_detail(sb: &mut String, key: &str, value_a: Option<String>, value_b: Option<String>) {
    let value_a = value_a.unwrap_or_else(|| "(null)".to_owned());
    let value_b = value_b.unwrap_or_else(|| "(null)".to_owned());

    sb.push_str(key);
    sb.push_str(": ");
    sb.push_str(&value_a);
    if value_a != value_b {
        sb.push_str(", ");
        sb.push_str(&value_b);
    }
    sb.push('\n');
}

// Java: com/google/devtools/build/lib/actions/ActionConflictException.java:162 ActionConflictException.addListDetail
fn add_list_detail(sb: &mut String, key: &str, value_a: &[Artifact], value_b: &[Artifact]) {
    let diff_a = difference_without_owner(value_a, value_b);
    let diff_b = difference_without_owner(value_b, value_a);

    sb.push_str(key);
    sb.push_str(": ");
    if diff_a.is_empty() && diff_b.is_empty() {
        sb.push_str("are equal\n");
    } else {
        if !diff_a.is_empty() {
            sb.push_str(&format!(
                "Attempted action contains artifacts not in previous action (first {}): \n",
                MAX_DIFF_ARTIFACTS_TO_REPORT
            ));
            pretty_print_artifact_diffs(sb, &diff_a);
        }

        if !diff_b.is_empty() {
            sb.push_str(&format!(
                "Previous action contains artifacts not in attempted action (first {}): \n",
                MAX_DIFF_ARTIFACTS_TO_REPORT
            ));
            pretty_print_artifact_diffs(sb, &diff_b);
        }
    }
}

/// Returns items in `value_a` that are not in `value_b`, ignoring the owner.
// Java: com/google/devtools/build/lib/actions/ActionConflictException.java:191 ActionConflictException.differenceWithoutOwner
fn difference_without_owner(value_a: &[Artifact], value_b: &[Artifact]) -> Vec<Artifact> {
    // ImmutableSet.Builder: distinct elements (Artifact.equals), in insertion order.
    let mut diff: Vec<Artifact> = Vec::new();

    // Group valueB by exec path for easier checks.
    let mut map_b: IndexMap<&str, Vec<&Artifact>> = IndexMap::new();
    for b in value_b {
        map_b.entry(b.get_exec_path_string()).or_default().push(b);
    }
    for a in value_a {
        let mut found = false;
        if let Some(candidates) = map_b.get(a.get_exec_path_string()) {
            for b in candidates {
                if a.equals_without_owner(b) {
                    found = true;
                    break;
                }
            }
        }
        if !found && !diff.contains(a) {
            diff.push(a.clone());
        }
    }

    diff
}

/// Pretty print action diffs (at most `MAX_DIFF_ARTIFACTS_TO_REPORT` lines).
// Java: com/google/devtools/build/lib/actions/ActionConflictException.java:216 ActionConflictException.prettyPrintArtifactDiffs
fn pretty_print_artifact_diffs(sb: &mut String, diff: &[Artifact]) {
    for artifact in diff.iter().take(MAX_DIFF_ARTIFACTS_TO_REPORT) {
        sb.push('\t');
        sb.push_str(&artifact.pretty_print());
        sb.push('\n');
    }
}

/// `action.getClass().toString()`: `class <binary name of the Java class>`.
///
/// PORT: a Rust action does not know the name of its Java class; the classes of `blaze_actions`
/// and of this crate (`TestRunnerAction`) are listed here. An action of another crate
/// (`CppCompileAction`, `CopyWithoutCachingAction`, the workspace status action) or a native
/// `SpawnAction` (which may be Java's `SpawnAction` or its subclass `GenRuleAction`) fails loudly
/// until `Action` has an accessor for it.
// Java: com/google/devtools/build/lib/actions/ActionConflictException.java:245 ActionConflictException.debugSuffix
fn java_action_class(action: &ActionRef) -> String {
    const ANALYSIS_ACTIONS: &str = "com.google.devtools.build.lib.analysis.actions";
    let any = action.as_any();
    let name = if let Some(file_write_action) = any.downcast_ref::<FileWriteAction>() {
        // FileWriteAction is abstract; `FileWriteAction.create` instantiates one of its two
        // nested classes.
        // Java: com/google/devtools/build/lib/analysis/actions/FileWriteAction.java:196 FileWriteAction.createInternal
        if file_write_action.is_compressed() {
            format!("{ANALYSIS_ACTIONS}.FileWriteAction$CompressedFileWriteAction")
        } else {
            format!("{ANALYSIS_ACTIONS}.FileWriteAction$RegularFileWriteAction")
        }
    } else if let Some(spawn_action) = any.downcast_ref::<SpawnAction>() {
        match spawn_action.get_class() {
            SpawnActionClass::Starlark => format!("{ANALYSIS_ACTIONS}.StarlarkAction"),
            SpawnActionClass::EnhancedStarlark(_) => {
                format!("{ANALYSIS_ACTIONS}.StarlarkAction$EnhancedStarlarkAction")
            }
            SpawnActionClass::Spawn => blaze_util::unported!(
                "ActionAnalysisMetadata.getClass",
                "the Java class of a native SpawnAction (mnemonic {})",
                action.get_mnemonic()
            ),
        }
    } else if any.is::<SymlinkAction>() {
        format!("{ANALYSIS_ACTIONS}.SymlinkAction")
    } else if any.is::<TemplateExpansionAction>() {
        format!("{ANALYSIS_ACTIONS}.TemplateExpansionAction")
    } else if any.is::<ParameterFileWriteAction>() {
        format!("{ANALYSIS_ACTIONS}.ParameterFileWriteAction")
    } else if any.is::<BinaryFileWriteAction>() {
        format!("{ANALYSIS_ACTIONS}.BinaryFileWriteAction")
    } else if any.is::<SymlinkTreeAction>() {
        format!("{ANALYSIS_ACTIONS}.SymlinkTreeAction")
    } else if any.is::<LazyWritePathsFileAction>() {
        format!("{ANALYSIS_ACTIONS}.LazyWritePathsFileAction")
    } else if any.is::<LazyWriteNestedSetOfTupleAction>() {
        format!("{ANALYSIS_ACTIONS}.LazyWriteNestedSetOfTupleAction")
    } else if any.is::<BuildInfoFileWriteAction>() {
        format!("{ANALYSIS_ACTIONS}.BuildInfoFileWriteAction")
    } else if any.is::<SourceManifestAction>() {
        "com.google.devtools.build.lib.analysis.SourceManifestAction".to_owned()
    } else if any.is::<RepoMappingManifestAction>() {
        "com.google.devtools.build.lib.analysis.RepoMappingManifestAction".to_owned()
    } else if any.is::<UnresolvedSymlinkAction>() {
        "com.google.devtools.build.lib.analysis.starlark.UnresolvedSymlinkAction".to_owned()
    } else if any.is::<RunfilesTreeAction>() {
        "com.google.devtools.build.lib.actions.RunfilesTreeAction".to_owned()
    } else if any.is::<crate::test::test_runner_action::TestRunnerAction>() {
        "com.google.devtools.build.lib.analysis.test.TestRunnerAction".to_owned()
    } else {
        blaze_util::unported!(
            "ActionAnalysisMetadata.getClass",
            "the Java class of the action with mnemonic {}",
            action.get_mnemonic()
        )
    };
    format!("class {name}")
}

/// See also `Actions.canBeShared()`.
// Java: com/google/devtools/build/lib/actions/ActionConflictException.java:224 ActionConflictException.debugSuffix
fn debug_suffix(action_key_context: &ActionKeyContext, a: &ActionRef, b: &ActionRef) -> String {
    // Note: the error message reveals to users the names of intermediate files that are not
    // documented in the BUILD language.  This error-reporting logic is rather elaborate but it
    // does help to diagnose some tricky situations.
    let mut sb = String::new();
    // PORT: `Action::get_owner` is never null in the port (Java tolerates a null owner here).
    let a_owner = a.get_owner();
    let b_owner = b.get_owner();

    add_string_detail(
        &mut sb,
        "Label",
        Some(label_print(a_owner.get_label())),
        Some(label_print(b_owner.get_label())),
    );
    if !a_owner.get_aspect_descriptors().is_empty() || !b_owner.get_aspect_descriptors().is_empty()
    {
        add_string_detail(
            &mut sb,
            "Aspects",
            Some(aspect_descriptor(a_owner)),
            Some(aspect_descriptor(b_owner)),
        );
    }
    add_string_detail(
        &mut sb,
        "RuleClass",
        Some(a_owner.get_target_kind().to_owned()),
        Some(b_owner.get_target_kind().to_owned()),
    );
    add_string_detail(
        &mut sb,
        "JavaActionClass",
        Some(java_action_class(a)),
        Some(java_action_class(b)),
    );
    add_string_detail(
        &mut sb,
        "Configuration",
        Some(a_owner.get_configuration_checksum().to_owned()),
        Some(b_owner.get_configuration_checksum().to_owned()),
    );
    add_string_detail(
        &mut sb,
        "Mnemonic",
        Some(a.get_mnemonic()),
        Some(b.get_mnemonic()),
    );
    add_string_detail(
        &mut sb,
        "IsShareable",
        Some(a.is_shareable().to_string()),
        Some(b.is_shareable().to_string()),
    );
    match (
        a.get_key(action_key_context, /* input_metadata_provider= */ None),
        b.get_key(action_key_context, /* input_metadata_provider= */ None),
    ) {
        (Ok(a_key), Ok(b_key)) => {
            add_string_detail(&mut sb, "Action key", Some(a_key), Some(b_key))
        }
        // Only for debugging - skip the key and carry on.
        _ => add_string_detail(
            &mut sb,
            "Action key",
            Some("<elided due to interrupt>".to_owned()),
            Some("<elided due to interrupt>".to_owned()),
        ),
    }

    // PORT: every ported action is an `ActionExecutionMetadata`.
    add_string_detail(
        &mut sb,
        "Progress message",
        a.get_progress_message(None),
        b.get_progress_message(None),
    );
    add_string_detail(
        &mut sb,
        "Action describeKey",
        a.describe_key(),
        b.describe_key(),
    );

    let a_primary_input = a.get_primary_input();
    let b_primary_input = b.get_primary_input();
    add_string_detail(
        &mut sb,
        "PrimaryInput",
        a_primary_input.as_ref().map(|i| i.to_string()),
        b_primary_input.as_ref().map(|i| i.to_string()),
    );
    add_string_detail(
        &mut sb,
        "PrimaryOutput",
        Some(a.get_primary_output().to_string()),
        Some(b.get_primary_output().to_string()),
    );

    // Only add list details if the primary input of A matches the input of B. Otherwise
    // the above information is enough and list diff detail is not needed.
    let primary_inputs_match = match (&a_primary_input, &b_primary_input) {
        (None, None) => true,
        (Some(a_primary_input), Some(b_primary_input)) => {
            a_primary_input.to_string() == b_primary_input.to_string()
        }
        _ => false,
    };
    if primary_inputs_match {
        let a_primary_output = a.get_primary_output();
        let b_primary_output = b.get_primary_output();
        if !a_primary_output.equals_without_owner(&b_primary_output) {
            // PORT: Java prints `System.identityHashCode` of the two artifacts: an arbitrary
            // number per object that differs from run to run. A hash of the artifact stands in.
            sb.push_str(&format!(
                "Primary outputs are different: {}, {}\n",
                identity_hash_code(&a_primary_output),
                identity_hash_code(&b_primary_output)
            ));
        }
        let a_artifact_owner = a_primary_output.get_artifact_owner();
        let b_artifact_owner = b_primary_output.get_artifact_owner();
        add_string_detail(
            &mut sb,
            "Owner information",
            Some(a_artifact_owner.to_string()),
            Some(b_artifact_owner.to_string()),
        );
        add_list_detail(
            &mut sb,
            "MandatoryInputs",
            &a.get_mandatory_inputs().to_list(),
            &b.get_mandatory_inputs().to_list(),
        );
        add_list_detail(&mut sb, "Outputs", &a.get_outputs(), &b.get_outputs());
    }

    sb
}

/// Stand-in for `System.identityHashCode(artifact)` (see its use).
fn identity_hash_code(artifact: &Artifact) -> i32 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    artifact.hash(&mut hasher);
    (hasher.finish() as u32 >> 1) as i32
}

// Java: com/google/devtools/build/lib/actions/ActionConflictException.java:321 ActionConflictException.aspectDescriptor
fn aspect_descriptor(owner: &blaze_actions::ActionOwner) -> String {
    format!(
        "[{}]",
        owner
            .get_aspect_descriptors()
            .iter()
            .map(|d| d.description.as_str())
            .collect::<Vec<_>>()
            .join(",")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // Java: com/google/devtools/build/lib/actions/ActionConflictException.java:151 ActionConflictException.addStringDetail
    #[test]
    fn add_string_detail_prints_one_value_when_equal_and_null_as_text() {
        let mut sb = String::new();
        add_string_detail(&mut sb, "Label", Some("//a:a".into()), Some("//a:a".into()));
        add_string_detail(&mut sb, "Label", Some("//a:a".into()), Some("//a:b".into()));
        add_string_detail(&mut sb, "PrimaryInput", None, None);
        add_string_detail(&mut sb, "PrimaryInput", None, Some("File:x".into()));
        assert_eq!(
            sb,
            "Label: //a:a\nLabel: //a:a, //a:b\nPrimaryInput: (null)\nPrimaryInput: (null), \
             File:x\n"
        );
    }

    // Java: com/google/devtools/build/lib/cmdline/Label.java:691 Label.print
    #[test]
    fn label_print_of_null_is_unknown() {
        assert_eq!(label_print(None), "(unknown)");
        assert_eq!(
            label_print(Some(&Label::parse_canonical_unchecked("//a:b"))),
            "//a:b"
        );
    }
}
