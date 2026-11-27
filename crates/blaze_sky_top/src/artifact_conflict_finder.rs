//! Ports of `skyframe/ArtifactConflictFinder.java`, `actions/MapBasedActionGraph.java` and the
//! conflict half of `actions/Actions.java` (`findArtifactPrefixConflicts`,
//! `isRunfilesArtifactPair`, `canBeSharedLogForPotentialFalsePositives`).
//!
//! PORT: `MapBasedActionGraph` and `Actions` belong to `lib/actions` (`blaze_actions`); they are
//! kept here, with their only caller, until that crate takes them.

use std::collections::HashMap;

use blaze_actions::actions::artifact::OwnerlessArtifactWrapper;
use blaze_actions::{ActionKeyContext, ActionRef, Artifact};
use blaze_skyframe::Interrupted;
use blaze_vfs::PathFragment;

use crate::action_conflict_exception::{ActionConflictException, ActionConflicts};

/// What `MutableActionGraph.registerAction` throws.
#[derive(Debug)]
pub enum RegisterActionError {
    /// `ActionConflictException`.
    ActionConflict(ActionConflictException),
    /// `InterruptedException`.
    Interrupted,
}

/// An action graph that resolves generating actions by looking them up in a map.
// Java: com/google/devtools/build/lib/actions/MapBasedActionGraph.java:24 MapBasedActionGraph
// Java: com/google/devtools/build/lib/actions/MutableActionGraph.java:22 MutableActionGraph
pub struct MapBasedActionGraph<'a> {
    action_key_context: &'a ActionKeyContext,
    generating_action_map: HashMap<OwnerlessArtifactWrapper, ActionRef>,
}

impl<'a> MapBasedActionGraph<'a> {
    // Java: com/google/devtools/build/lib/actions/MapBasedActionGraph.java:29 MapBasedActionGraph.<init>
    pub fn new(action_key_context: &'a ActionKeyContext) -> Self {
        Self::with_size_hint(action_key_context, /* size_hint= */ 16)
    }

    // Java: com/google/devtools/build/lib/actions/MapBasedActionGraph.java:33 MapBasedActionGraph.<init>
    pub fn with_size_hint(action_key_context: &'a ActionKeyContext, size_hint: usize) -> Self {
        MapBasedActionGraph {
            action_key_context,
            generating_action_map: HashMap::with_capacity(size_hint),
        }
    }

    // Java: com/google/devtools/build/lib/actions/MapBasedActionGraph.java:40 MapBasedActionGraph.getGeneratingAction
    pub fn get_generating_action(&self, artifact: &Artifact) -> Option<&ActionRef> {
        self.generating_action_map
            .get(&OwnerlessArtifactWrapper(artifact.clone()))
    }

    /// Attempts to register the action. If any of the action's outputs already has a generating
    /// action, and the two actions are not compatible, then an `ActionConflictException` is
    /// thrown. The internal data structure may be partially modified when that happens; it is
    /// not guaranteed that all potential conflicts are detected, but at least one of them is.
    // Java: com/google/devtools/build/lib/actions/MapBasedActionGraph.java:46 MapBasedActionGraph.registerAction
    pub fn register_action(&mut self, action: &ActionRef) -> Result<(), RegisterActionError> {
        register_action_in(
            &mut self.generating_action_map,
            self.action_key_context,
            action,
        )
    }

    // Java: com/google/devtools/build/lib/actions/MapBasedActionGraph.java:65 MapBasedActionGraph.getSize
    pub fn get_size(&self) -> i32 {
        self.generating_action_map.len() as i32
    }
}

/// `MapBasedActionGraph.registerAction` over a generating-action map and an action key context
/// (the map of a [`MapBasedActionGraph`], or the incremental conflict finder's shared map).
// Java: com/google/devtools/build/lib/actions/MapBasedActionGraph.java:46 MapBasedActionGraph.registerAction
pub(crate) fn register_action_in(
    generating_action_map: &mut HashMap<OwnerlessArtifactWrapper, ActionRef>,
    action_key_context: &ActionKeyContext,
    action: &ActionRef,
) -> Result<(), RegisterActionError> {
    for artifact in action.get_outputs() {
        // putIfAbsent
        let previous_action =
            match generating_action_map.entry(OwnerlessArtifactWrapper(artifact.clone())) {
                std::collections::hash_map::Entry::Vacant(e) => {
                    e.insert(action.clone());
                    None
                }
                std::collections::hash_map::Entry::Occupied(e) => Some(e.get().clone()),
            };
        if let Some(previous_action) = previous_action
            && !std::sync::Arc::ptr_eq(&previous_action, action)
        {
            if can_be_shared_log_for_potential_false_positives(
                action_key_context,
                action,
                &previous_action,
            )? {
                // All outputs can be shared. No need to register the remaining outputs.
                return Ok(());
            }
            // TODO(blaze-configurability-team): May be possible to do some inspection here
            // and provide advice on possible known-common reasons for conflicts.
            // e.g. If only config diffs where one is missing TestOptions:
            //   --trim_test_configuration or, if the --platforms differ but have same
            //   shortname
            return Err(RegisterActionError::ActionConflict(
                ActionConflictException::create(
                    action_key_context,
                    &artifact,
                    &previous_action,
                    action,
                ),
            ));
        }
    }
    Ok(())
}

impl From<blaze_collect::command_line_item::InterruptedException> for RegisterActionError {
    fn from(_: blaze_collect::command_line_item::InterruptedException) -> Self {
        RegisterActionError::Interrupted
    }
}

/// Checks if two actions are equivalent (`Actions.canBeShared`).
///
/// PORT: skipped, logging: Java logs "Shared action ... has a tree artifact input" at most every
/// five minutes.
// Java: com/google/devtools/build/lib/actions/Actions.java:125 Actions.canBeSharedLogForPotentialFalsePositives
fn can_be_shared_log_for_potential_false_positives(
    action_key_context: &ActionKeyContext,
    action_a: &ActionRef,
    action_b: &ActionRef,
) -> Result<bool, blaze_collect::command_line_item::InterruptedException> {
    blaze_analysis::rule::actions_util::can_be_shared(action_key_context, action_a, action_b)
}

/// The conflicts `findAndStoreArtifactConflicts` found and the size of the action graph.
// Java: com/google/devtools/build/lib/skyframe/ArtifactConflictFinder.java:145 ArtifactConflictFinder.ActionConflictsAndStats
pub struct ActionConflictsAndStats {
    conflicts: ActionConflicts,
    output_artifact_count: i32,
}

impl ActionConflictsAndStats {
    // Java: com/google/devtools/build/lib/skyframe/ArtifactConflictFinder.java:148 ArtifactConflictFinder.ActionConflictsAndStats.create
    pub(crate) fn create(conflicts: ActionConflicts, output_artifact_count: i32) -> Self {
        ActionConflictsAndStats {
            conflicts,
            output_artifact_count,
        }
    }

    pub fn conflicts(&self) -> &ActionConflicts {
        &self.conflicts
    }

    pub fn output_artifact_count(&self) -> i32 {
        self.output_artifact_count
    }
}

/// Find conflicts between generated artifacts. There are two ways to have conflicts. First, if
/// two (unshareable) actions generate the same output artifact, this will result in an
/// `ActionConflictException`. Second, if one action generates an artifact whose path is a prefix
/// of another artifact's path, those two artifacts cannot exist simultaneously in the output
/// tree. This causes an `ActionConflictException`.
///
/// This method must be called if a new action was added to the graph this build, so whenever a
/// new configured target was analyzed this build. It is somewhat expensive (~1s range for a
/// medium build as of 2014), so it should only be called when necessary.
///
/// `action_lookup_values` is `Sharder<ActionLookupValue>`: the actions of each
/// `ActionLookupValue` of the build.
///
/// PORT: Java registers the shards on a thread pool, so which of two conflicting actions is the
/// "previous" and which the "attempted" one (and with it the label in the keep_going warning)
/// depends on thread timing. The port registers the values one after the other, in the order of
/// the traversal that collected them.
// Java: com/google/devtools/build/lib/skyframe/ArtifactConflictFinder.java:61 ArtifactConflictFinder.findAndStoreArtifactConflicts
pub fn find_and_store_artifact_conflicts(
    action_lookup_values: &[Vec<ActionRef>],
    action_count: i32,
    action_key_context: &ActionKeyContext,
) -> Result<ActionConflictsAndStats, Interrupted> {
    let mut temporary_bad_action_map = ActionConflicts::new();

    // Use the action count to presize - all actions have at least one output artifact.
    let mut action_graph =
        MapBasedActionGraph::with_size_hint(action_key_context, action_count.max(0) as usize);
    let mut artifacts: Vec<Artifact> = Vec::with_capacity(action_count.max(0) as usize);

    construct_action_graph_and_artifact_list(
        &mut action_graph,
        &mut artifacts,
        action_lookup_values,
        &mut temporary_bad_action_map,
    )?;

    let actions_with_artifact_prefix_conflict =
        find_artifact_prefix_conflicts(&action_graph, &artifacts);
    for (action, exception) in actions_with_artifact_prefix_conflict.entries() {
        temporary_bad_action_map.put(action, exception.clone());
    }
    Ok(ActionConflictsAndStats {
        conflicts: temporary_bad_action_map,
        output_artifact_count: action_graph.get_size(),
    })
}

/// Simultaneously construct an action graph for all the actions in Skyframe and a map from
/// `PathFragment`s to their respective `Artifact`s.
///
/// PORT: one thread (see `find_and_store_artifact_conflicts`); `actionRegistration` is the loop
/// body.
// Java: com/google/devtools/build/lib/skyframe/ArtifactConflictFinder.java:93 ArtifactConflictFinder.constructActionGraphAndArtifactList
// Java: com/google/devtools/build/lib/skyframe/ArtifactConflictFinder.java:112 ArtifactConflictFinder.actionRegistration
fn construct_action_graph_and_artifact_list(
    action_graph: &mut MapBasedActionGraph<'_>,
    all_artifacts: &mut Vec<Artifact>,
    values: &[Vec<ActionRef>],
    bad_action_map: &mut ActionConflicts,
) -> Result<(), Interrupted> {
    for value in values {
        for action in value {
            match action_graph.register_action(action) {
                Ok(()) => {}
                Err(RegisterActionError::ActionConflict(e)) => {
                    // It may be possible that we detect a conflict for the same action more than
                    // once, if that action belongs to multiple aspect values. In this case we
                    // will harmlessly overwrite the badActionMap entry.
                    bad_action_map.put(action, e);
                    // We skip the rest of the loop, and do not add the path->artifact mapping
                    // for this artifact below -- we don't need to check it since this action is
                    // already in error.
                    continue;
                }
                Err(RegisterActionError::Interrupted) => {
                    // Bail.
                    return Err(Interrupted);
                }
            }
            all_artifacts.extend(action.get_outputs());
        }
    }
    Ok(())
}

// Java: com/google/devtools/build/lib/actions/Actions.java:288 Actions.EXEC_PATH_PREFIX_COMPARATOR
fn exec_path_prefix_comparator(a: &Artifact, b: &Artifact) -> std::cmp::Ordering {
    PathFragment::hierarchical_compare(a.get_exec_path(), b.get_exec_path())
}

/// Check whether two artifacts are a runfiles tree - runfiles output manifest pair.
///
/// This is necessary because these are exempt from the "path of one artifact cannot be a prefix
/// of another" rule. This is like this for historical reasons.
// Java: com/google/devtools/build/lib/actions/Actions.java:297 Actions.isRunfilesArtifactPair
pub fn is_runfiles_artifact_pair(runfiles_tree: &Artifact, runfiles_manifest: &Artifact) -> bool {
    if !runfiles_tree.is_runfiles_tree() {
        // The outside artifact is not a runfiles tree. No go.
        return false;
    }

    // Now check whether the path of the inner artifact matches the expected path of a runfiles
    // output manifest.
    runfiles_manifest.get_exec_path_string()
        == runfiles_tree
            .get_exec_path()
            .get_relative_str("MANIFEST")
            .get_path_string()
}

/// Finds Artifact prefix conflicts between generated artifacts. An artifact prefix conflict
/// happens if one action generates an artifact whose path is a strict prefix of another
/// artifact's path. Those two artifacts cannot exist simultaneously in the output tree.
///
/// - `action_graph`: the `ActionGraph` to query for artifact conflicts
/// - `artifacts`: all generated artifacts in the build
///
/// Returns a map between actions that generated the conflicting artifacts and their associated
/// `ActionConflictException`.
// Java: com/google/devtools/build/lib/actions/Actions.java:320 Actions.findArtifactPrefixConflicts
pub fn find_artifact_prefix_conflicts(
    action_graph: &MapBasedActionGraph<'_>,
    artifacts: &[Artifact],
) -> ActionConflicts {
    // No actions in graph -- currently happens only in tests. Special-cased because .next() call
    // below is unconditional.
    if artifacts.is_empty() {
        return ActionConflicts::new();
    }

    let mut artifact_array: Vec<Artifact> = artifacts.to_vec();
    // Arrays.parallelSort: a stable sort.
    artifact_array.sort_by(exec_path_prefix_comparator);

    // Keep deterministic ordering of bad actions.
    let mut bad_actions = ActionConflicts::new();
    let mut iter = artifact_array.iter().peekable();

    // Report an error for every derived artifact which is a strict prefix of another.
    // If x << y << z (where x << y means "y starts with x"), then we only report (x,y), (x,z),
    // but not (y,z).
    let mut artifact_j = iter.next().expect("non-empty");
    while iter.peek().is_some() {
        // For each comparison, we have a prefix candidate (pathI) and a suffix candidate (pathJ).
        // At the beginning of the loop, we set pathI to the last suffix candidate, since it has
        // not yet been tested as a prefix candidate, and then set pathJ to the paths coming after
        // pathI, until we come to one that does not contain pathI as a prefix. pathI is then
        // verified not to be the prefix of any path, so we start the next run of the loop.
        let artifact_i = artifact_j;
        let path_i = artifact_i.get_exec_path();
        // Compare pathI to the paths coming after it.
        // Java: while (iter.hasNext()) { artifactJ = iter.next(); ... }
        for next in iter.by_ref() {
            artifact_j = next;
            let path_j = artifact_j.get_exec_path();
            // Check length first so that we only detect strict prefix conflicts. Equal exec paths
            // are possible from shared actions.
            if path_j.get_path_string().len() > path_i.get_path_string().len()
                && path_j.starts_with(path_i)
                && !is_runfiles_artifact_pair(artifact_i, artifact_j)
            {
                let action_i = action_graph
                    .get_generating_action(artifact_i)
                    .unwrap_or_else(|| panic!("NullPointerException: {}", artifact_i));
                let action_j = action_graph
                    .get_generating_action(artifact_j)
                    .unwrap_or_else(|| panic!("NullPointerException: {}", artifact_j));
                let exception = ActionConflictException::create_prefix(
                    artifact_i, artifact_j, action_i, action_j,
                );
                bad_actions.put(action_i, exception.clone());
                bad_actions.put(action_j, exception);
            } else {
                // pathJ didn't have prefix pathI, so no conflict possible for pathI.
                break;
            }
        }
    }
    bad_actions
}

#[cfg(test)]
mod tests {
    // Java: src/test/java/com/google/devtools/build/lib/actions/MapBasedActionGraphTest.java
    // PORT: the tests register `TestAction`s (lib/actions/util/TestAction.java) over artifacts
    // made by `ActionsTestUtil`; neither test helper is ported.

    #[test]
    #[ignore = "needs TestAction and ActionsTestUtil.createArtifact (actions test utilities)"]
    fn test_smoke() {
        unimplemented!("not ported: MapBasedActionGraphTest.testSmoke")
    }

    #[test]
    #[ignore = "needs TestAction and ActionsTestUtil.createArtifact (actions test utilities)"]
    fn test_no_action_conflict_when_unregistering_shared_action() {
        unimplemented!(
            "not ported: MapBasedActionGraphTest.testNoActionConflictWhenUnregisteringSharedAction"
        )
    }

    #[test]
    #[ignore = "needs TestAction and ActionsTestUtil.createArtifact (actions test utilities)"]
    fn test_shared_action_stress_test() {
        unimplemented!("not ported: MapBasedActionGraphTest.testSharedActionStressTest")
    }

    #[test]
    #[ignore = "needs TestAction and ActionsTestUtil.createArtifact (actions test utilities)"]
    fn test_action_conflict_shows_is_shareable() {
        unimplemented!("not ported: MapBasedActionGraphTest.testActionConflictShowsIsShareable")
    }
}
