//! Port of `lib/skyframe/ActionTemplateExpansionFunction.java`.
//!
//! PORT: the key and value (`ActionTemplateExpansionValue`) are in `blaze_sky_exec`, which
//! `ArtifactFunction` and `ActionExecutionFunction` need; the function is here because it uses
//! the analysis-side conflict checks (`Actions.assignOwnersAndThrowIfConflict`,
//! `Actions.findArtifactPrefixConflicts`).

use std::sync::Arc;

use blaze_actions::actions::action::ActionRef;
use blaze_actions::{ActionKeyContext, ActionLookupKeyRef, Artifact, ArtifactOwner};
use blaze_analysis::rule::actions_util::{AssignOwnersError, assign_owners_and_throw_if_conflict};
use blaze_events::event::Event;
use blaze_events::handler::EventHandler;
use blaze_exec::actions::action_adapters::as_blaze_artifact;
use blaze_exec::actions::{
    ActionExecutionException, ActionTemplate, ActionTemplateError, as_action_template,
};
use blaze_sky_exec::action_template_expansion_value::{
    ActionTemplateExpansionKey, ActionTemplateExpansionValue,
};
use blaze_sky_exec::analysis_seam::AnalysisSeam;
use blaze_sky_exec::artifact_function::ArtifactValue;
use blaze_sky_exec::keys::{ArtifactKey, ArtifactNestedSetKey, artifact_key};
use blaze_sky_exec::skyframe_action_executor::SkyframeActionExecutor;
use blaze_skyframe::{
    ComputeError, Environment, Interrupted, SkyFunctionException, SkyKey, Transience,
};
use blaze_util::check_state;

use crate::action_conflict_exception::ActionConflictException;
use crate::artifact_conflict_finder::{
    MapBasedActionGraph, RegisterActionError, find_artifact_prefix_conflicts,
};

/// The SkyFunction for `ActionTemplateExpansionValue`.
///
/// Given an action template, this function resolves its input TreeArtifact, then expands the
/// action template into a list of actions using the expanded `TreeFileArtifact`s under the input
/// TreeArtifact.
// Java: com/google/devtools/build/lib/skyframe/ActionTemplateExpansionFunction.java:55 ActionTemplateExpansionFunction
pub struct ActionTemplateExpansionFunction {
    /// Holds the `ActionKeyContext` (`getActionKeyContext()`).
    pub skyframe_action_executor: Arc<SkyframeActionExecutor>,
    pub seam: Arc<dyn AnalysisSeam>,
}

/// What `compute` throws: `ActionConflictException` or an (already reported)
/// `ActionExecutionException`, both PERSISTENT.
// Java: com/google/devtools/build/lib/skyframe/ActionTemplateExpansionFunction.java:136 ActionTemplateExpansionFunction.ActionTemplateExpansionFunctionException
fn action_template_expansion_function_exception<E: std::error::Error + Send + Sync + 'static>(
    e: E,
) -> ComputeError {
    SkyFunctionException::new(e, Transience::Persistent).into()
}

impl ActionTemplateExpansionFunction {
    // Java: com/google/devtools/build/lib/skyframe/ActionTemplateExpansionFunction.java:66 ActionTemplateExpansionFunction.compute
    pub fn compute(
        &self,
        key: &ActionTemplateExpansionKey,
        env: &mut Environment<'_>,
    ) -> Result<Option<ActionTemplateExpansionValue>, ComputeError> {
        let Some(value) = self
            .seam
            .get_action_lookup_value(key.get_action_lookup_key(), env)?
        else {
            // Because of the phase boundary separating analysis and execution, all needed
            // ActionLookupValues must have already been evaluated, so a missing
            // ActionLookupValue is unexpected. However, we tolerate this case.
            // PORT: skipped, tracing (BugReport.sendBugReport).
            return Ok(None);
        };
        // ActionLookupValue.getActionTemplate(index)
        let action_template_ref = get_action_template(&value.get_actions()[..], key);
        let action_template = as_action_template(action_template_ref.as_ref())
            .unwrap_or_else(|| unreachable!("checked by get_action_template"));
        let mut input_keys: Vec<SkyKey> = action_template
            .get_input_tree_artifacts()
            .into_iter()
            .map(|a| SkyKey::new(ArtifactKey(a)))
            .collect();
        // Following b/143205147, we unwrap the top layer of the NestedSet and evaluate the first
        // layer of the NestedSet as direct Artifact(s) and transitive NestedSet(s).
        let inputs = action_template.get_inputs();
        if !inputs.is_empty() {
            for leaf in inputs.get_leaves() {
                input_keys.push(artifact_key(&leaf));
            }
            for non_leaf in inputs.get_non_leaves() {
                input_keys.push(SkyKey::new(ArtifactNestedSetKey::create(non_leaf)));
            }
        }
        let result = env.get_values_and_exceptions(&input_keys)?;
        // Input TreeArtifact is not ready yet.
        if env.values_missing() {
            return Ok(None);
        }
        let actions: Vec<ActionRef>;
        let expansion: Result<Vec<ActionRef>, ExpansionError> = (|| {
            let mut input_tree_file_artifacts: Vec<Artifact> = Vec::new();
            for input_tree_artifact in action_template.get_input_tree_artifacts() {
                let tree_artifact_value = result
                    .get_or_throw::<ActionExecutionException>(&SkyKey::new(ArtifactKey(
                        input_tree_artifact.clone(),
                    )))
                    .map_err(ExpansionError::ActionExecution)?
                    .unwrap_or_else(|| panic!("NullPointerException: {input_tree_artifact}"));
                let tree_artifact_value = tree_artifact_value
                    .downcast::<ArtifactValue>()
                    .unwrap_or_else(|_| panic!("ClassCastException: {input_tree_artifact}"));
                let ArtifactValue::Tree(tree_artifact_value) = &*tree_artifact_value else {
                    panic!("ClassCastException: {input_tree_artifact} is not a TreeArtifactValue");
                };
                for child in tree_artifact_value.get_children() {
                    input_tree_file_artifacts.push(
                        as_blaze_artifact(&child)
                            .unwrap_or_else(|| panic!("ClassCastException: {child}"))
                            .clone(),
                    );
                }
            }
            // Expand the action template using the list of expanded input TreeFileArtifacts.
            // TODO(rduan): Add a check to verify the inputs of expanded actions are subsets of
            // inputs of the ActionTemplate.
            generate_and_validate_actions_from_template(
                action_template,
                input_tree_file_artifacts,
                key,
                env.listener(),
            )
        })();
        match expansion {
            Ok(a) => actions = a,
            Err(ExpansionError::ActionExecution(e)) => {
                env.listener().handle(&Event::error_at(
                    Some(action_template.get_owner().get_location().clone()),
                    format!("{} failed: {}", action_template.describe(), e.get_message()),
                ));
                return Err(action_template_expansion_function_exception(
                    ActionExecutionException::already_reported(e),
                ));
            }
            Err(ExpansionError::Interrupted) => return Err(ComputeError::Interrupted(Interrupted)),
        }

        let action_key_context = self.skyframe_action_executor.get_action_key_context();
        match check_action_and_artifact_conflicts(action_key_context, &actions, key) {
            Ok(()) => {}
            Err(CheckError::ActionConflict(e)) => {
                e.report_to(env.listener());
                return Err(action_template_expansion_function_exception(e));
            }
            Err(CheckError::ArtifactGeneratedByOtherRule(e)) => {
                panic!(
                    "IllegalStateException: Actions generated by template {} did not all output \
                     tree file artifacts belonging to the correct output tree artifact + ({:?}): {}",
                    action_template.describe(),
                    key,
                    e
                );
            }
            Err(CheckError::Interrupted) => return Err(ComputeError::Interrupted(Interrupted)),
        }
        Ok(Some(ActionTemplateExpansionValue::new(actions)))
    }
}

/// `ActionLookupValue.getActionTemplate(index)`.
// Java: com/google/devtools/build/lib/actions/ActionLookupValue.java:35 ActionLookupValue.getActionTemplate
fn get_action_template(actions: &[ActionRef], key: &ActionTemplateExpansionKey) -> ActionRef {
    let index = key.get_action_index();
    let result = actions[index as usize].clone();
    if as_action_template(result.as_ref()).is_none() {
        panic!(
            "IllegalStateException: Not action template: {} at {}",
            result.to_string_java(),
            index
        );
    }
    result
}

enum ExpansionError {
    ActionExecution(ActionExecutionException),
    Interrupted,
}

impl From<ActionTemplateError> for ExpansionError {
    fn from(e: ActionTemplateError) -> Self {
        match e {
            ActionTemplateError::ActionExecution(e) => ExpansionError::ActionExecution(e),
            ActionTemplateError::Interrupted => ExpansionError::Interrupted,
        }
    }
}

// Java: com/google/devtools/build/lib/skyframe/ActionTemplateExpansionFunction.java:146 ActionTemplateExpansionFunction.generateAndValidateActionsFromTemplate
fn generate_and_validate_actions_from_template(
    action_template: &dyn ActionTemplate,
    input_tree_file_artifacts: Vec<Artifact>,
    key: &ActionTemplateExpansionKey,
    event_handler: &dyn EventHandler,
) -> Result<Vec<ActionRef>, ExpansionError> {
    let outputs = action_template.get_outputs();
    for output in &outputs {
        check_state!(
            output.is_tree_artifact(),
            "{} declares an output which is not a tree artifact: {}",
            action_template.to_string_java(),
            output
        );
    }
    let owner = ActionLookupKeyRef::new(key.clone());
    let actions = action_template.generate_actions_for_input_artifacts(
        input_tree_file_artifacts,
        owner.clone(),
        event_handler,
    )?;
    for action in &actions {
        for output in action.get_outputs() {
            check_state!(
                output.get_artifact_owner() == ArtifactOwner::ActionLookupKey(owner.clone()),
                "{} generated an action with an output owned by the wrong owner {} not {} ({})",
                action_template.to_string_java(),
                output.get_artifact_owner(),
                key,
                action.to_string_java()
            );
            check_state!(
                output.has_parent(),
                "{} generated an action which outputs a non-TreeFileArtifact {} ({})",
                action_template.to_string_java(),
                output,
                action.to_string_java()
            );
            check_state!(
                outputs.contains(output.get_parent().expect("has_parent")),
                "{} generated an action with an output {} under an undeclared tree not in {:?} ({})",
                action_template.to_string_java(),
                output,
                outputs,
                action.to_string_java()
            );
        }
    }
    Ok(actions)
}

enum CheckError {
    ActionConflict(ActionConflictException),
    ArtifactGeneratedByOtherRule(String),
    Interrupted,
}

// Java: com/google/devtools/build/lib/skyframe/ActionTemplateExpansionFunction.java:186 ActionTemplateExpansionFunction.checkActionAndArtifactConflicts
fn check_action_and_artifact_conflicts(
    action_key_context: &ActionKeyContext,
    actions: &[ActionRef],
    key: &ActionTemplateExpansionKey,
) -> Result<(), CheckError> {
    match assign_owners_and_throw_if_conflict(
        action_key_context,
        actions,
        &ActionLookupKeyRef::new(key.clone()),
    ) {
        Ok(()) => {}
        Err(AssignOwnersError::ActionConflict(e)) => return Err(CheckError::ActionConflict(e)),
        Err(AssignOwnersError::ArtifactGeneratedByOtherRule(e)) => {
            return Err(CheckError::ArtifactGeneratedByOtherRule(e));
        }
        Err(AssignOwnersError::Interrupted) => return Err(CheckError::Interrupted),
    }
    // getMapForConsistencyCheck(actions) as a MapBasedImmutableActionGraph, then
    // findArtifactPrefixConflicts(graph, map.keySet()).
    // PORT: Java's map is a `HashMap<Artifact, ActionAnalysisMetadata>` where a later action
    // replaces an earlier one with the same output; `assignOwnersAndThrowIfConflict` above has
    // already rejected two actions with one output, so registering in order builds the same
    // graph. The key set's hash order does not matter: findArtifactPrefixConflicts sorts it.
    let mut graph = MapBasedActionGraph::new(action_key_context);
    let mut artifacts: Vec<Artifact> = Vec::new();
    for action in actions {
        match graph.register_action(action) {
            Ok(()) => {}
            Err(RegisterActionError::ActionConflict(e)) => {
                return Err(CheckError::ActionConflict(e));
            }
            Err(RegisterActionError::Interrupted) => return Err(CheckError::Interrupted),
        }
        artifacts.extend(action.get_outputs());
    }
    let artifact_prefix_conflict_map = find_artifact_prefix_conflicts(&graph, &artifacts);
    if let Some((_, e)) = artifact_prefix_conflict_map.entries().next() {
        return Err(CheckError::ActionConflict(e.clone()));
    }
    Ok(())
}
