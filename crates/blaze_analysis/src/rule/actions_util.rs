//! The parts of `actions/Actions.java` that configured-target creation runs
//! (`assignOwnersAndThrowIfConflictToleratingSharedActions` and `canBeShared`).
//!
//! PORT: move to `blaze_actions::actions` (lead-owned); kept here with the configured-target
//! builder that calls them.

use std::collections::HashMap;

use blaze_actions::actions::runfiles_tree_action::RunfilesTreeAction;
use blaze_actions::{ActionKeyContext, ActionLookupData, ActionLookupKeyRef, ActionRef, Artifact};
use blaze_collect::command_line_item::InterruptedException;
use blaze_util::check_state;
use blaze_vfs::PathFragment;

use crate::test::test_runner_action::TestRunnerAction;

/// Errors of owner assignment (Java `ActionConflictException`,
/// `Actions.ArtifactGeneratedByOtherRuleException`, `InterruptedException`).
#[derive(Debug, thiserror::Error)]
pub enum AssignOwnersError {
    /// Java `ActionConflictException` (message).
    #[error("{0}")]
    ActionConflict(String),
    /// Java `Actions.ArtifactGeneratedByOtherRuleException` (message).
    #[error("{0}")]
    ArtifactGeneratedByOtherRule(String),
    #[error("interrupted")]
    Interrupted,
}

impl From<InterruptedException> for AssignOwnersError {
    fn from(_: InterruptedException) -> Self {
        AssignOwnersError::Interrupted
    }
}

/// Checks if the action depends on the build ID (volatile actions and actions notified on
/// action-cache hits).
// Java: com/google/devtools/build/lib/actions/Actions.java:60 Actions.dependsOnBuildId
pub fn depends_on_build_id(action: &ActionRef) -> bool {
    // Volatile build actions may need to execute even if none of their known inputs have
    // changed. Depending on the build ID ensures that these actions have a chance to execute.
    // PORT: `NotifyOnActionCacheHit` is implemented by TestRunnerAction only among ported
    // actions.
    if action.as_any().is::<TestRunnerAction>() {
        return true;
    }
    action.is_volatile()
}

/// Checks if two actions are equivalent. This method exists to support sharing actions between
/// configured targets for cases where there is no canonical target that could own the action.
// Java: com/google/devtools/build/lib/actions/Actions.java:85 Actions.canBeShared
pub fn can_be_shared(
    action_key_context: &ActionKeyContext,
    a: &ActionRef,
    b: &ActionRef,
) -> Result<bool, InterruptedException> {
    if !(a.get_mnemonic() == b.get_mnemonic()
        && a.get_key(action_key_context, None)? == b.get_key(action_key_context, None)?)
    {
        return Ok(false);
    }
    // Uses a standard comparison technique for shareable actions.
    if a.is_shareable() && b.is_shareable() {
        return Ok(artifacts_equal_without_owner(
            &a.get_mandatory_inputs().to_list(),
            &b.get_mandatory_inputs().to_list(),
        ) && artifacts_equal_without_owner(&a.get_outputs(), &b.get_outputs()));
    }
    // If this is reached, at least one action is not shareable. If the actions differ on this,
    // they cannot be shared.
    if a.is_shareable() || b.is_shareable() {
        return Ok(false);
    }
    // If the artifacts are in fact equal (with owners), these are aliases of the same action and
    // not in conflict with each other.
    Ok(
        *a.get_mandatory_inputs().to_list() == *b.get_mandatory_inputs().to_list()
            && a.get_outputs() == b.get_outputs(),
    )
}

// Java: com/google/devtools/build/lib/actions/Actions.java:146 Actions.artifactsEqualWithoutOwner
fn artifacts_equal_without_owner(collection1: &[Artifact], collection2: &[Artifact]) -> bool {
    if collection1.len() != collection2.len() {
        return false;
    }
    collection1
        .iter()
        .zip(collection2)
        .all(|(a1, a2)| a1.equals_without_owner(a2))
}

// Java: com/google/devtools/build/lib/actions/Actions.java:202 Actions.verifyGeneratingActionKeys
fn verify_generating_action_keys(
    output: &Artifact,
    other_key: &ActionLookupData,
    allow_shared_action: bool,
    action_key_context: &ActionKeyContext,
    actions: &[ActionRef],
) -> Result<(), AssignOwnersError> {
    let first_key = output.get_generating_action_key();
    check_state!(
        first_key.get_action_lookup_key() == other_key.get_action_lookup_key(),
        "Mismatched lookup keys? {} {} {}",
        output,
        first_key,
        other_key
    );
    let action_index = first_key.get_action_index() as usize;
    let other_index = other_key.get_action_index() as usize;
    if action_index != other_index
        && (!allow_shared_action
            || !can_be_shared(
                action_key_context,
                &actions[action_index],
                &actions[other_index],
            )?)
    {
        // PORT: ActionConflictException.create formats a detailed diff of the two actions.
        blaze_util::unported!("ActionConflictException.create");
    }
    Ok(())
}

/// Assigns `ActionLookupData` keys to the outputs of `actions` (in registration order) and
/// throws if two actions generate the same output and cannot be shared.
// Java: com/google/devtools/build/lib/actions/Actions.java:191 Actions.assignOwnersAndThrowIfConflictToleratingSharedActions
pub fn assign_owners_and_throw_if_conflict_tolerating_shared_actions(
    action_key_context: &ActionKeyContext,
    actions: &[ActionRef],
    action_lookup_key: &ActionLookupKeyRef,
) -> Result<(), AssignOwnersError> {
    assign_owners_and_throw_if_conflict_maybe_tolerating_shared_actions(
        action_key_context,
        actions,
        action_lookup_key,
        /* allow_shared_action= */ true,
    )
}

// Java: com/google/devtools/build/lib/actions/Actions.java:226 Actions.assignOwnersAndThrowIfConflictMaybeToleratingSharedActions
fn assign_owners_and_throw_if_conflict_maybe_tolerating_shared_actions(
    action_key_context: &ActionKeyContext,
    actions: &[ActionRef],
    action_lookup_key: &ActionLookupKeyRef,
    allow_shared_action: bool,
) -> Result<(), AssignOwnersError> {
    let mut seen_artifacts: HashMap<PathFragment, Artifact> = HashMap::new();
    // Loop over the actions, looking at all outputs for conflicts.
    for (action_index, action) in actions.iter().enumerate() {
        // Runfiles tree actions have the unfortunate property that their RichArtifactData
        // contains a NestedSet of Artifacts, which we currently deem to be not worth serializing.
        let generating_action_key =
            if depends_on_build_id(action) || action.as_any().is::<RunfilesTreeAction>() {
                ActionLookupData::create_unshareable(action_lookup_key.clone(), action_index as i32)
            } else {
                ActionLookupData::create(action_lookup_key.clone(), action_index as i32)
            };
        for output in action.get_outputs() {
            check_state!(
                !output.is_source_artifact(),
                "Source in outputs: {} {} {:?}",
                output,
                generating_action_key,
                action
            );
            // Has an artifact with this execPath been seen before?
            let equal_output = match seen_artifacts.get(output.get_exec_path()) {
                Some(e) => Some(e.clone()),
                None => {
                    seen_artifacts.insert(output.get_exec_path().clone(), output.clone());
                    None
                }
            };
            if let Some(equal_output) = equal_output {
                // Yes: assert that its generating action and this artifact's are compatible.
                verify_generating_action_keys(
                    &equal_output,
                    &generating_action_key,
                    allow_shared_action,
                    action_key_context,
                    actions,
                )?;
            }
            // Was this output already seen, so it has a generating action key set?
            if !output.has_generating_action_key() {
                // Common case: artifact hasn't been seen before.
                output.set_generating_action_key(generating_action_key.clone());
            } else {
                let old_key = output.get_generating_action_key();
                if action_lookup_key != old_key.get_action_lookup_key() {
                    // The rule is claiming to produce an output that one of its inputs
                    // produced. Silly!
                    return Err(AssignOwnersError::ArtifactGeneratedByOtherRule(format!(
                        "File '{}' is produced by {} but is already generated by rule {}",
                        output.pretty_print(),
                        action.pretty_print(),
                        old_key
                            .get_action_lookup_key()
                            .get_label()
                            .map(|l| l.to_string())
                            .unwrap_or_else(|| "null".to_owned())
                    )));
                }
                // Key is already set: verify that the generating action and this action are
                // compatible.
                verify_generating_action_keys(
                    &output,
                    &generating_action_key,
                    allow_shared_action,
                    action_key_context,
                    actions,
                )?;
            }
        }
    }
    Ok(())
}
