//! `CompletionFunction` with `TargetCompletor`: the `TARGET_COMPLETION` SkyFunction.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use blaze_actions::Artifact;
use blaze_collect::{NestedSet, NestedSetBuilder, NestedSetNode};
use blaze_exec::actions::action_adapters::as_blaze_artifact;
use blaze_exec::actions::{ActionExecutionException, FileArtifactValue, TreeArtifactValue};
use blaze_skyframe::{ComputeError, Environment, SkyFunctionException, SkyKey, Transience};
use indexmap::IndexSet;

use crate::action_execution_value::ActionExecutionValue;
use crate::analysis_seam::{AnalysisSeam, ConfiguredTargetView};
use crate::artifact_function::{ArtifactValue, SourceArtifactException};
use crate::events::{ArtifactsInOutputGroup, CompletionRootCause, TargetCompleteEvent};
use crate::keys::{TargetCompletionKey, artifact_key};
use crate::skyframe_action_executor::SkyframeActionExecutor;

/// The (marker) value of a completed top-level target.
// Java: com/google/devtools/build/lib/skyframe/TargetCompletionValue.java:33 TargetCompletionValue
#[derive(Clone, Debug, Default)]
pub struct TargetCompletionValue;

/// The output groups of a target as `TopLevelArtifactHelper.getAllArtifactsToBuild` sees them:
/// each requested group present in the target's `OutputGroupInfo`, with its artifacts.
// Java: com/google/devtools/build/lib/analysis/TopLevelArtifactHelper.java:192 TopLevelArtifactHelper.getAllArtifactsToBuild
pub fn get_all_artifacts_to_build(
    target: &dyn ConfiguredTargetView,
    output_groups: &[String],
) -> Vec<(String, Vec<Artifact>)> {
    let groups = target.get_output_groups();
    let mut result = Vec::new();
    for name in output_groups {
        if let Some(set) = groups.get(name) {
            result.push((name.clone(), set.to_list().as_ref().clone()));
        }
    }
    result
}

/// An input file problem of a top-level target (`InputFileErrorException`, a
/// `BuildFailedException` whose error was already shown).
// Java: com/google/devtools/build/lib/actions/InputFileErrorException.java:23 InputFileErrorException
#[derive(Clone, Debug)]
pub struct InputFileErrorException {
    pub message: String,
    /// `getDetailedExitCode()`: the most important code of the source-file causes.
    pub detailed_exit_code: blaze_util::DetailedExitCode,
}

impl std::fmt::Display for InputFileErrorException {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for InputFileErrorException {}

/// `ActionExecutionFunction.createSourceErrorCodeAndMessage`: the most important exit code of
/// the source-file causes and the message `N input file(s) are in error or do not exist`.
// Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:1543 ActionExecutionFunction.createSourceErrorCodeAndMessage
pub fn create_source_error_code_and_message(
    source_artifact_error_causes: &[CompletionRootCause],
) -> (blaze_util::DetailedExitCode, String) {
    let failure_details: Vec<&blaze_protos::failure_details::FailureDetail> =
        source_artifact_error_causes
            .iter()
            .map(|cause| match cause {
                CompletionRootCause::ActionFailed { failure_detail, .. }
                | CompletionRootCause::Label { failure_detail, .. } => failure_detail,
            })
            .collect();
    create_source_error_code_and_message_of(&failure_details)
}

/// [`create_source_error_code_and_message`] over `Cause.getDetailedExitCode()` of each cause.
// Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:1543 ActionExecutionFunction.createSourceErrorCodeAndMessage
pub(crate) fn create_source_error_code_and_message_of(
    source_artifact_error_causes: &[&blaze_protos::failure_details::FailureDetail],
) -> (blaze_util::DetailedExitCode, String) {
    use blaze_protos::failure_details::{execution, failure_detail::Category};
    let mut saw_source_artifact_exception = false;
    let mut saw_missing_file = false;
    let mut prioritized_detailed_exit_code: Option<blaze_util::DetailedExitCode> = None;
    for failure_detail in source_artifact_error_causes {
        match &failure_detail.category {
            Some(Category::Filesystem(_)) => saw_source_artifact_exception = true,
            Some(Category::Execution(e))
                if e.code == execution::Code::SourceInputIoException as i32 =>
            {
                saw_source_artifact_exception = true
            }
            Some(Category::Execution(e))
                if e.code == execution::Code::SourceInputMissing as i32 =>
            {
                saw_missing_file = true
            }
            // BugReport.sendNonFatalBugReport("Unexpected error code in ..."): logging only.
            _ => {}
        }
        // Stream.max(DetailedExitCodeComparator.INSTANCE): the first of equally important codes.
        prioritized_detailed_exit_code =
            blaze_util::DetailedExitCode::choose_more_important_with_first_if_tie(
                prioritized_detailed_exit_code,
                Some(blaze_util::DetailedExitCode::of((*failure_detail).clone())),
            );
    }
    let parts: Vec<&str> = [
        saw_source_artifact_exception.then_some("are in error"),
        saw_missing_file.then_some("do not exist"),
    ]
    .into_iter()
    .flatten()
    .collect();
    let error_message = format!(
        "{} input file(s) {}",
        source_artifact_error_causes.len(),
        parts.join(" or ")
    );
    (
        prioritized_detailed_exit_code
            .unwrap_or_else(|| panic!("NoSuchElementException: no source error causes")),
        error_message,
    )
}

/// `CompletionFunction.handleSourceFileError`: records the input's `LabelCause` and reports
/// `TargetCompletor.getRootCauseError` (`<label>: <cause message>` at the target's location).
// Java: com/google/devtools/build/lib/skyframe/CompletionFunction.java:346 CompletionFunction.handleSourceFileError
fn handle_source_file_error(
    skyframe_action_executor: &SkyframeActionExecutor,
    input: &Artifact,
    failure_detail: &blaze_protos::failure_details::FailureDetail,
    root_causes_builder: &mut Vec<CompletionRootCause>,
    env: &Environment<'_>,
    key_label: &blaze_cmdline::Label,
    location: &blaze_events::Location,
) {
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:1160 ActionExecutionFunction.createLabelCauseNullOwnerOk
    let cause = CompletionRootCause::Label {
        label: input.get_owner().unwrap_or_else(|| key_label.clone()),
        failure_detail: failure_detail.clone(),
    };
    push_unique(root_causes_builder, cause);
    // Java: com/google/devtools/build/lib/skyframe/TargetCompletor.java:65 TargetCompletor.getRootCauseError
    // LabelCause.getMessage(): the failure detail's message.
    env.listener().handle(&blaze_events::Event::error_at(
        Some(location.clone()),
        format!("{}: {}", key_label, failure_detail.message),
    ));
    skyframe_action_executor.record_execution_error();
}

/// `OutputGroupInfo.HIDDEN_OUTPUT_GROUP_PREFIX`.
// Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:85 OutputGroupInfo.HIDDEN_OUTPUT_GROUP_PREFIX
const HIDDEN_OUTPUT_GROUP_PREFIX: &str = "_";

/// `TopLevelArtifactHelper.getAllArtifactsToBuild(target, context).getAllArtifactsByOutputGroup()`:
/// each requested group the target has, with its `NestedSet` as built and whether it is
/// important.
// Java: com/google/devtools/build/lib/analysis/TopLevelArtifactHelper.java:204 TopLevelArtifactHelper.getAllArtifactsToBuild
fn get_all_artifacts_by_output_group(
    target: &dyn ConfiguredTargetView,
    output_groups: &[String],
) -> Vec<(String, ArtifactsInOutputGroup)> {
    let groups = target.get_output_groups();
    let mut all_output_groups = Vec::new();
    for output_group in output_groups {
        // The view leaves out groups that have no artifacts.
        let Some(artifacts) = groups.get(output_group) else {
            continue;
        };
        let is_important_group = !output_group.starts_with(HIDDEN_OUTPUT_GROUP_PREFIX);
        all_output_groups.push((
            output_group.clone(),
            ArtifactsInOutputGroup {
                artifacts: artifacts.clone(),
                important: is_important_group,
                incomplete: false,
            },
        ));
    }
    all_output_groups
}

/// The value of one input of the completion: `ArtifactFunction`'s value (source and tree
/// artifacts) or the generating action's `ActionExecutionValue`.
enum InputValue {
    Artifact(ArtifactValue),
    Action(Arc<ActionExecutionValue>),
}

/// The `ActionInputMap` the completion builds for its important artifacts, as the event carries
/// it.
#[derive(Default)]
struct ImportantInputMap {
    files: Vec<(Artifact, FileArtifactValue)>,
    trees: Vec<(
        Artifact,
        FileArtifactValue,
        Vec<(Artifact, FileArtifactValue)>,
    )>,
}

impl ImportantInputMap {
    // Java: com/google/devtools/build/lib/actions/ActionInputMap.java:99 ActionInputMap.put
    fn put(&mut self, artifact: &Artifact, metadata: FileArtifactValue) {
        self.files.push((artifact.clone(), metadata));
    }
}

// Java: com/google/devtools/build/lib/skyframe/ActionInputMapHelper.java:107 ActionInputMapHelper.expandTreeArtifactAndPopulateArtifactData
fn expand_tree_artifact_and_populate_artifact_data(
    tree_artifact: &Artifact,
    value: &TreeArtifactValue,
    input_map: &mut ImportantInputMap,
) {
    // inputMap.putTreeArtifact(treeArtifact, value)
    let metadata = value
        .metadata
        .clone()
        .unwrap_or_else(|| panic!("NullPointerException: metadata of {tree_artifact}"));
    let children = value
        .child_values
        .iter()
        .map(|(child, child_metadata)| {
            let child = as_blaze_artifact(child)
                .unwrap_or_else(|| {
                    panic!("ClassCastException: a child of {tree_artifact} is not an Artifact")
                })
                .clone();
            (child, child_metadata.clone())
        })
        .collect();
    input_map
        .trees
        .push((tree_artifact.clone(), metadata, children));
    let Some((archived_tree_file_artifact, archived_file_value)) = &value.archived_representation
    else {
        return;
    };
    let archived = as_blaze_artifact(archived_tree_file_artifact)
        .unwrap_or_else(|| panic!("ClassCastException: the archived artifact of {tree_artifact}"));
    input_map.put(archived, archived_file_value.clone());
}

/// `ActionInputMapHelper.addToMap` for the values a completion reads.
// Java: com/google/devtools/build/lib/skyframe/ActionInputMapHelper.java:31 ActionInputMapHelper.addToMap
fn add_to_map(input_map: &mut ImportantInputMap, key: &Artifact, value: &InputValue) {
    match value {
        InputValue::Artifact(ArtifactValue::Tree(tree_artifact_value)) => {
            expand_tree_artifact_and_populate_artifact_data(key, tree_artifact_value, input_map);
        }
        InputValue::Artifact(ArtifactValue::File(file_artifact_value)) => {
            input_map.put(key, file_artifact_value.clone());
        }
        InputValue::Artifact(ArtifactValue::Missing(_)) => {
            panic!("IllegalStateException: Unexpected value {key}")
        }
        InputValue::Action(action_execution_value) => {
            // Actions resulting from the expansion of an ActionTemplate consume only one of the
            // files in a tree artifact. However, the input prefetcher and the Linux sandbox
            // require access to the tree metadata to determine the prefetch location of a tree
            // artifact materialized as a symlink to another (cf.
            // TreeArtifactValue#getResolvedPath()).
            if key.is_child_of_declared_directory() {
                let tree_artifact = key
                    .get_parent()
                    .unwrap_or_else(|| panic!("NullPointerException: parent of {key}"));
                let tree_artifact_value = action_execution_value
                    .get_tree_artifact_value(tree_artifact)
                    .unwrap_or_else(|| panic!("NullPointerException: {tree_artifact}"));
                expand_tree_artifact_and_populate_artifact_data(
                    tree_artifact,
                    tree_artifact_value,
                    input_map,
                );
            }
            if key.is_fileset() {
                blaze_util::unported!("ActionInputMapHelper.addToMap (filesets)");
            } else if key.is_runfiles_tree() {
                let runfiles_artifact_value = action_execution_value
                    .get_rich_artifact_data()
                    .unwrap_or_else(|| panic!("NullPointerException: rich artifact data of {key}"));
                // Note: we don't expand the .runfiles/MANIFEST file into the inputs. The reason
                // for that being that the MANIFEST file contains absolute paths that don't work
                // with remote execution.
                // Instead, the way the SpawnInputExpander expands runfiles is via the Runfiles
                // class which contains all artifacts in the runfiles tree minus the MANIFEST file.
                runfiles_artifact_value.for_each_file(|artifact, metadata| {
                    let artifact = as_blaze_artifact(artifact).unwrap_or_else(|| {
                        panic!("ClassCastException: {artifact} of {key} is not an Artifact")
                    });
                    input_map.put(artifact, metadata.clone());
                });
                runfiles_artifact_value.for_each_tree(|tree_artifact, metadata| {
                    let tree_artifact = as_blaze_artifact(tree_artifact).unwrap_or_else(|| {
                        panic!("ClassCastException: {tree_artifact} of {key} is not an Artifact")
                    });
                    expand_tree_artifact_and_populate_artifact_data(
                        tree_artifact,
                        metadata,
                        input_map,
                    );
                });
                // No filesets (see RunfilesArtifactValue). inputMap.putRunfilesMetadata(key,
                // runfilesArtifactValue): the event's view of the map carries files and trees,
                // which is all that CompletionContext reads.
            } else {
                let metadata = action_execution_value
                    .get_existing_file_artifact_value(key)
                    .cloned()
                    .unwrap_or_else(|| panic!("NullPointerException: metadata of {key}"));
                input_map.put(key, metadata);
            }
        }
    }
}

/// Recursive procedure filtering a target's declared output groups to only include artifacts
/// that were produced by successful actions.
// Java: com/google/devtools/build/lib/analysis/TopLevelArtifactHelper.java:297 TopLevelArtifactHelper.SuccessfulArtifactFilter
struct SuccessfulArtifactFilter<'a> {
    artifact_set_can_be_skipped: HashSet<NestedSetNode<Artifact>>,
    artifact_set_to_filtered_set: HashMap<NestedSetNode<Artifact>, NestedSet<Artifact>>,
    built_artifacts: &'a HashSet<Artifact>,
}

impl<'a> SuccessfulArtifactFilter<'a> {
    fn new(built_artifacts: &'a HashSet<Artifact>) -> Self {
        SuccessfulArtifactFilter {
            artifact_set_can_be_skipped: HashSet::new(),
            artifact_set_to_filtered_set: HashMap::new(),
            built_artifacts,
        }
    }

    /// Filters the declared output groups to only include artifacts that were actually built.
    // Java: com/google/devtools/build/lib/analysis/TopLevelArtifactHelper.java:312 SuccessfulArtifactFilter.filterArtifactsInOutputGroup
    fn filter_artifacts_in_output_group(
        &mut self,
        output_groups: Vec<(String, ArtifactsInOutputGroup)>,
    ) -> Vec<(String, ArtifactsInOutputGroup)> {
        let mut leaves_dirty = false;
        let mut result_builder = Vec::new();
        for (name, artifacts_in_output_group) in &output_groups {
            let filtered_artifacts_in_output_group = match self
                .filter_artifact_nested_set_to_built_artifacts(&artifacts_in_output_group.artifacts)
            {
                None => artifacts_in_output_group.clone(),
                Some(filtered_artifacts) => {
                    leaves_dirty = true;
                    ArtifactsInOutputGroup {
                        artifacts: filtered_artifacts,
                        important: artifacts_in_output_group.important,
                        incomplete: true,
                    }
                }
            };
            if !filtered_artifacts_in_output_group.artifacts.is_empty() {
                result_builder.push((name.clone(), filtered_artifacts_in_output_group));
            }
        }
        if !leaves_dirty {
            return output_groups;
        }
        result_builder
    }

    /// Recursively filters the declared artifacts to only include artifacts that were actually
    /// built. Returns `None` if no artifacts are filtered out of the input.
    // Java: com/google/devtools/build/lib/analysis/TopLevelArtifactHelper.java:346 SuccessfulArtifactFilter.filterArtifactNestedSetToBuiltArtifacts
    fn filter_artifact_nested_set_to_built_artifacts(
        &mut self,
        declared_artifacts: &NestedSet<Artifact>,
    ) -> Option<NestedSet<Artifact>> {
        let declared_artifacts_node = declared_artifacts.to_node();
        if self
            .artifact_set_can_be_skipped
            .contains(&declared_artifacts_node)
        {
            return None;
        }
        if let Some(memoized_filtered_set) = self
            .artifact_set_to_filtered_set
            .get(&declared_artifacts_node)
        {
            return Some(memoized_filtered_set.clone());
        }

        // Scan the Artifact leaves for any artifact not present in builtArtifacts. If an un-built
        // artifact is found, exit the loop early, and construct the list of filteredArtifacts
        // later. This avoids unnecessary allocation in the case where all artifacts are built.
        let leaves = declared_artifacts.get_leaves();
        let leaves_dirty = leaves.iter().any(|a| !self.built_artifacts.contains(a));
        // Unconditionally populate filteredNonLeaves by filtering each NestedSet<Artifact>
        // non-leaf successor, and set nonLeavesDirty if anything is filtered out.
        let mut non_leaves_dirty = false;
        let non_leaves = declared_artifacts.get_non_leaves();
        let mut filtered_non_leaves = Vec::with_capacity(non_leaves.len());
        for non_leaf in non_leaves {
            let filtered_non_leaf =
                match self.filter_artifact_nested_set_to_built_artifacts(&non_leaf) {
                    // None indicates no filtering happened and the input may be used as-is.
                    None => non_leaf,
                    Some(filtered) => {
                        non_leaves_dirty = true;
                        filtered
                    }
                };
            if !filtered_non_leaf.is_empty() {
                filtered_non_leaves.push(filtered_non_leaf);
            }
        }
        if !leaves_dirty && !non_leaves_dirty {
            self.artifact_set_can_be_skipped
                .insert(declared_artifacts_node);
            // Returning None indicates no filtering happened and the input may be used as-is.
            return None;
        }
        let mut new_set_builder = NestedSetBuilder::new_builder(declared_artifacts.get_order());
        for a in leaves {
            if self.built_artifacts.contains(&a) {
                new_set_builder.add(a);
            }
        }
        for filtered_non_leaf in &filtered_non_leaves {
            new_set_builder.add_transitive(filtered_non_leaf);
        }
        let result = new_set_builder.build();
        self.artifact_set_to_filtered_set
            .insert(declared_artifacts_node, result.clone());
        Some(result)
    }
}

/// `CompletionFunction.compute` for targets.
// Java: com/google/devtools/build/lib/skyframe/CompletionFunction.java:153 CompletionFunction.compute
pub fn compute_target_completion(
    skyframe_action_executor: &SkyframeActionExecutor,
    seam: &dyn AnalysisSeam,
    key: &TargetCompletionKey,
    env: &mut Environment<'_>,
) -> Result<Option<TargetCompletionValue>, ComputeError> {
    // Java: com/google/devtools/build/lib/skyframe/TargetCompletor.java:66 TargetCompletor.getValueAndArtifactsToBuild
    let Some(value) = seam.get_configured_target(&key.action_lookup_key, env)? else {
        return Ok(None);
    };
    let artifacts_to_build = get_all_artifacts_to_build(&*value, &key.output_groups);
    let artifacts_by_output_group = get_all_artifacts_by_output_group(&*value, &key.output_groups);
    if env.values_missing() {
        return Ok(None);
    }
    let all_artifacts: IndexSet<Artifact> = artifacts_to_build
        .iter()
        .flat_map(|(_, a)| a.iter().cloned())
        .collect();
    let keys: Vec<SkyKey> = all_artifacts.iter().map(artifact_key).collect();
    let input_deps = env.get_values_and_exceptions(&keys)?;

    // ArtifactsToBuild.areAllOutputGroupsImportant() / getImportantArtifacts().
    // Java: com/google/devtools/build/lib/analysis/TopLevelArtifactHelper.java:130 ArtifactsToBuild.areAllOutputGroupsImportant
    let all_artifacts_are_important = artifacts_by_output_group
        .iter()
        .all(|(_, group)| group.important);
    // Prepare an ActionInputMap for important artifacts separately, to be used by BEP events.
    let important_artifacts: IndexSet<Artifact> = if all_artifacts_are_important {
        IndexSet::new()
    } else {
        artifacts_by_output_group
            .iter()
            .filter(|(_, group)| group.important)
            .flat_map(|(_, group)| group.artifacts.to_list().as_ref().clone())
            .collect()
    };
    let mut important_input_map = ImportantInputMap::default();
    let mut built_artifacts: HashSet<Artifact> = HashSet::new();
    // key.actionLookupKey().getLabel().
    let key_label = key
        .action_lookup_key
        .get_label()
        .unwrap_or_else(|| panic!("NullPointerException: label of {}", key.action_lookup_key));
    // TargetCompletor.getLocationIdentifier: the target's location, read where Java reads it
    // (for an error or the source file warning).
    // Java: com/google/devtools/build/lib/skyframe/TargetCompletor.java:74 TargetCompletor.getLocationIdentifier
    let get_location_identifier = || value.get_location();
    // NestedSetBuilder<Cause>.stableOrder(): the exceptions' causes (transitive) come before the
    // label causes (direct) when the set is flattened; equal causes are kept once.
    let mut action_causes: Vec<CompletionRootCause> = Vec::new();
    let mut label_causes: Vec<CompletionRootCause> = Vec::new();

    let mut first_action_execution_exception: Option<ActionExecutionException> = None;
    let mut root_causes = Vec::new();
    for (input, k) in all_artifacts.iter().zip(&keys) {
        let catch = |e: &blaze_skyframe::SkyError| {
            if let Some(x) = e.downcast_ref::<ActionExecutionException>() {
                return Some(Err(x.clone()));
            }
            e.downcast_ref::<SourceArtifactException>()
                .map(|x| Ok(x.clone()))
        };
        let got = match k.downcast::<crate::keys::ArtifactKey>() {
            Some(ak) => input_deps
                .get_or_throw_with(ak, catch)
                .map(|v| v.map(|v| InputValue::Artifact((*v).clone()))),
            None => input_deps
                .get_or_throw_with(
                    k.downcast::<crate::keys::ActionExecutionKey>().unwrap(),
                    catch,
                )
                .map(|v| v.map(InputValue::Action)),
        };
        match got {
            Ok(None) => continue,
            Ok(Some(InputValue::Artifact(ArtifactValue::Missing(m)))) => {
                root_causes.push(input.get_owner());
                handle_source_file_error(
                    skyframe_action_executor,
                    input,
                    m.get_failure_detail(),
                    &mut label_causes,
                    env,
                    &key_label,
                    &get_location_identifier(),
                );
            }
            Ok(Some(input_value)) => {
                built_artifacts.insert(input.clone());
                if all_artifacts_are_important || important_artifacts.contains(input) {
                    add_to_map(&mut important_input_map, input, &input_value);
                }
            }
            Err(Err(e)) => {
                for c in e.get_root_causes() {
                    root_causes.push(Some(c.label.clone()));
                    // Java: com/google/devtools/build/lib/causes/ActionFailed.java:38 ActionFailed.<init>
                    // PORT: blaze_exec's ActionFailed does not keep its DetailedExitCode; the
                    // exception's failure detail stands for it (exact for the exception's own
                    // cause, the failing action's).
                    let failure_detail = e.get_failure_detail().cloned().unwrap_or_else(|| {
                        blaze_util::unported!(
                            "ActionFailed.getDetailedExitCode (no failure detail)"
                        )
                    });
                    push_unique(
                        &mut action_causes,
                        CompletionRootCause::ActionFailed {
                            exec_path: c.exec_path.clone(),
                            label: Some(c.label.clone()),
                            configuration_checksum: c.configuration_checksum.clone(),
                            failure_detail,
                        },
                    );
                }
                // Prefer a catastrophic exception as the one we propagate.
                let replace = match &first_action_execution_exception {
                    None => true,
                    Some(f) => !f.is_catastrophe() && e.is_catastrophe(),
                };
                if replace {
                    first_action_execution_exception = Some(e);
                }
            }
            Err(Ok(e)) => {
                root_causes.push(input.get_owner());
                handle_source_file_error(
                    skyframe_action_executor,
                    input,
                    e.get_failure_detail(),
                    &mut label_causes,
                    env,
                    &key_label,
                    &get_location_identifier(),
                );
            }
        }
    }

    if !root_causes.is_empty() || first_action_execution_exception.is_some() {
        // In error bubbling, we may be interrupted by Skyframe. Java ensures that the interrupt
        // doesn't prevent staging built artifacts (informImportantOutputHandler: remote outputs,
        // not ported) and posting the failed event (CompletionFunction.java:248). Nothing below
        // polls the interrupt (no `env.check_interrupted()?`), so the event is always posted.
        // Java: com/google/devtools/build/lib/skyframe/CompletionFunction.java:329 CompletionFunction.postFailedEvent
        let built_outputs = SuccessfulArtifactFilter::new(&built_artifacts)
            .filter_artifacts_in_output_group(artifacts_by_output_group);
        let mut root_cause_details = action_causes;
        for cause in &label_causes {
            push_unique(&mut root_cause_details, cause.clone());
        }
        env.listener().post(Arc::new(TargetCompleteEvent {
            label: value.get_label(),
            configuration_checksum: value.get_configuration_checksum(),
            root_causes: root_causes.iter().flatten().cloned().collect(),
            output_groups: artifacts_to_build.clone(),
            is_test: key.will_test,
            failed: true,
            artifacts_by_output_group: built_outputs,
            important_input_map: important_input_map.files,
            important_tree_artifacts: important_input_map.trees,
            root_cause_details: root_cause_details.clone(),
        }));
        if let Some(e) = first_action_execution_exception {
            // Java: com/google/devtools/build/lib/skyframe/CompletionFunction.java:467 CompletionFunctionException.<init>
            // isCatastrophic(): actionException != null && actionException.isCatastrophe().
            let catastrophic = e.is_catastrophe();
            return Err(SkyFunctionException {
                catastrophic,
                ..SkyFunctionException::new(e, Transience::Persistent)
            }
            .into());
        }
        // Java: com/google/devtools/build/lib/skyframe/CompletionFunction.java:282 CompletionFunction.compute
        // Without an ActionExecutionException every root cause is a source file's LabelCause.
        let (detailed_exit_code, message) =
            create_source_error_code_and_message(&root_cause_details);
        // The location identifier of a target is its Location (`locationPrefix instanceof
        // Location`), so the message carries no prefix.
        env.listener().handle(&blaze_events::Event::error_at(
            Some(get_location_identifier()),
            message.clone(),
        ));
        // CompletionFunctionException(InputFileErrorException): isCatastrophic() is false.
        return Err(SkyFunctionException::new(
            InputFileErrorException {
                message,
                detailed_exit_code,
            },
            Transience::Persistent,
        )
        .into());
    }

    // Only check for missing values *after* reporting errors.
    if env.values_missing() {
        return Ok(None);
    }

    // Java: com/google/devtools/build/lib/skyframe/TargetCompletor.java:104 TargetCompletor.createSucceeded
    // PORT: `target instanceof InputFileConfiguredTarget` is the view's kind `source file` (an
    // input file's target kind).
    if !key.will_test && value.get_rule_class_or_kind() == "source file" {
        env.listener().handle(&blaze_events::Event::warn_at(
            Some(get_location_identifier()),
            format!(
                "{} is a source file, nothing will be built for it. If you want to build a target \
                 that consumes this file, try --compile_one_dependency",
                value.get_label()
            ),
        ));
    }
    env.listener().post(Arc::new(TargetCompleteEvent {
        label: value.get_label(),
        configuration_checksum: value.get_configuration_checksum(),
        root_causes: Vec::new(),
        output_groups: artifacts_to_build,
        is_test: key.will_test,
        failed: false,
        artifacts_by_output_group,
        important_input_map: important_input_map.files,
        important_tree_artifacts: important_input_map.trees,
        root_cause_details: Vec::new(),
    }));
    Ok(Some(TargetCompletionValue))
}

/// `NestedSetBuilder.add`: an element already present is kept once.
fn push_unique(causes: &mut Vec<CompletionRootCause>, cause: CompletionRootCause) {
    if !causes.contains(&cause) {
        causes.push(cause);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blaze_protos::failure_details::{self, FailureDetail, execution, failure_detail::Category};

    fn label_cause(name: &str, category: Category) -> CompletionRootCause {
        CompletionRootCause::Label {
            label: blaze_cmdline::Label::parse_canonical_unchecked(&format!("//p:{name}")),
            failure_detail: FailureDetail {
                message: format!("missing input file '//p:{name}'"),
                category: Some(category),
            },
        }
    }

    fn execution_code(code: execution::Code) -> Category {
        Category::Execution(failure_details::Execution { code: code as i32 })
    }

    // Java: ActionExecutionFunction.createSourceErrorCodeAndMessage
    #[test]
    fn source_error_message_for_missing_files() {
        let (code, message) = create_source_error_code_and_message(&[
            label_cause("a.txt", execution_code(execution::Code::SourceInputMissing)),
            label_cause("b.txt", execution_code(execution::Code::SourceInputMissing)),
        ]);
        assert_eq!(message, "2 input file(s) do not exist");
        assert_eq!(
            code.get_failure_detail().map(|f| f.message.as_str()),
            Some("missing input file '//p:a.txt'")
        );
    }

    #[test]
    fn source_error_message_for_io_errors_and_missing_files() {
        let (_, message) = create_source_error_code_and_message(&[
            label_cause(
                "a.txt",
                execution_code(execution::Code::SourceInputIoException),
            ),
            label_cause("b.txt", execution_code(execution::Code::SourceInputMissing)),
        ]);
        assert_eq!(message, "2 input file(s) are in error or do not exist");
        let (_, message) = create_source_error_code_and_message(&[label_cause(
            "a.txt",
            Category::Filesystem(Default::default()),
        )]);
        assert_eq!(message, "1 input file(s) are in error");
    }
}
