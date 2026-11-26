//! Ports of `lib/skyframe/rewinding/RewindableGraphInconsistencyReceiver.java` and
//! `RewindingInconsistencyUtils.java`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

use blaze_skyframe::SkyKey;
use blaze_skyframe::graph_inconsistency_receiver::{GraphInconsistencyReceiver, Inconsistency};
use blaze_util::check_state;
use parking_lot::Mutex;

use crate::keys::{
    ActionExecutionKey, ArtifactKey, ArtifactNestedSetKey, AspectCompletionKey,
    TargetCompletionKey, TestCompletionKey,
};

// Java: com/google/devtools/build/lib/skyframe/rewinding/RewindableGraphInconsistencyReceiver.java:50 RewindableGraphInconsistencyReceiver.LOGGED_CHILDREN_LIMIT
const LOGGED_CHILDREN_LIMIT: usize = 50;

/// A [`GraphInconsistencyReceiver`] that tolerates inconsistencies resulting from action
/// rewinding, and fails on all others.
///
/// PORT: `heuristicallyDropNodes` and the Skymeld node-dropping inconsistencies
/// (`NodeDroppingInconsistencyReceiver`) are not ported: Blaze neither drops nodes heuristically
/// nor discards the analysis cache in a way that drops nodes the execution phase reads; both
/// flags are false.
// Java: com/google/devtools/build/lib/skyframe/rewinding/RewindableGraphInconsistencyReceiver.java:46 RewindableGraphInconsistencyReceiver
#[derive(Default)]
pub struct RewindableGraphInconsistencyReceiver {
    self_counts: Mutex<HashMap<Inconsistency, i64>>,
    child_counts: Mutex<HashMap<Inconsistency, i64>>,
    rewinding_initiated: AtomicBool,
}

impl RewindableGraphInconsistencyReceiver {
    // Java: com/google/devtools/build/lib/skyframe/rewinding/RewindableGraphInconsistencyReceiver.java:58 RewindableGraphInconsistencyReceiver.<init>
    pub fn new() -> RewindableGraphInconsistencyReceiver {
        RewindableGraphInconsistencyReceiver::default()
    }

    /// Returns whether this is the first time the inconsistency was noted for a parent.
    // Java: com/google/devtools/build/lib/skyframe/rewinding/RewindableGraphInconsistencyReceiver.java:175 RewindableGraphInconsistencyReceiver.noteSelfInconsistency
    fn note_self_inconsistency(&self, inconsistency: Inconsistency) -> bool {
        let mut counts = self.self_counts.lock();
        let count = counts.entry(inconsistency).or_insert(0);
        *count += 1;
        *count == 1
    }

    /// `getInconsistencyStats()`: the self and child counts.
    // Java: com/google/devtools/build/lib/skyframe/rewinding/RewindableGraphInconsistencyReceiver.java:200 RewindableGraphInconsistencyReceiver.getInconsistencyStats
    pub fn get_inconsistency_stats(
        &self,
    ) -> (HashMap<Inconsistency, i64>, HashMap<Inconsistency, i64>) {
        (self.self_counts.lock().clone(), self.child_counts.lock().clone())
    }

    fn add_child_count(&self, inconsistency: Inconsistency, count: usize) {
        *self.child_counts.lock().entry(inconsistency).or_insert(0) += count as i64;
    }
}

/// `listChildren(otherKeys)`: at most [`LOGGED_CHILDREN_LIMIT`] keys.
// Java: com/google/devtools/build/lib/skyframe/rewinding/RewindableGraphInconsistencyReceiver.java:184 RewindableGraphInconsistencyReceiver.listChildren
fn list_children(children: &[SkyKey]) -> String {
    let shown: Vec<String> = children
        .iter()
        .take(LOGGED_CHILDREN_LIMIT)
        .map(|k| format!("{k:?}"))
        .collect();
    if children.len() > LOGGED_CHILDREN_LIMIT {
        // Java: com/google/devtools/build/lib/util/StringUtil.java:93 StringUtil.listItemsWithLimit
        format!(
            "{} ...(omitting {} more item(s))",
            shown.join(", "),
            children.len() - LOGGED_CHILDREN_LIMIT
        )
    } else {
        format!("[{}]", shown.join(", "))
    }
}

// Java: com/google/devtools/build/lib/skyframe/rewinding/RewindableGraphInconsistencyReceiver.java:193 RewindableGraphInconsistencyReceiver.unexpectedInconsistency
fn unexpected_inconsistency(
    key: &SkyKey,
    other_keys: Option<&[SkyKey]>,
    inconsistency: Inconsistency,
) -> ! {
    let children = match other_keys {
        None => "null".to_owned(),
        Some(keys) => list_children(keys),
    };
    panic!(
        "IllegalStateException: Unexpected inconsistency {inconsistency}, key = {key:?}, \
         otherKeys = {children}"
    );
}

impl GraphInconsistencyReceiver for RewindableGraphInconsistencyReceiver {
    // Java: com/google/devtools/build/lib/skyframe/rewinding/RewindableGraphInconsistencyReceiver.java:64 RewindableGraphInconsistencyReceiver.noteInconsistencyAndMaybeThrow
    fn note_inconsistency_and_maybe_throw(
        &self,
        key: &SkyKey,
        other_keys: Option<&[SkyKey]>,
        inconsistency: Inconsistency,
    ) {
        // The following block categorizes inconsistencies that could happen because of
        // rewinding. RESET_REQUESTED and PARENT_FORCE_REBUILD_OF_CHILD may be the first
        // inconsistencies seen with rewinding. BUILDING_PARENT_FOUND_UNDONE_CHILD may also be
        // seen, but it will not be the first.
        match inconsistency {
            Inconsistency::ResetRequested => {
                check_state!(
                    is_type_that_depends_on_rewindable_nodes(key),
                    "Unexpected reset requested for: {:?}",
                    key
                );
                // PORT: skipped, logging ("Reset requested for: %s").
                self.note_self_inconsistency(inconsistency);
                self.rewinding_initiated.store(true, Ordering::SeqCst);
            }
            Inconsistency::ParentForceRebuildOfChild => {
                let other_keys = other_keys.expect("NullPointerException: otherKeys");
                let parent_may_force_rebuild_children = may_force_rebuild_children(key);
                let unrewindable_rebuild_children: Vec<SkyKey> = other_keys
                    .iter()
                    .filter(|k| !is_rewindable(k))
                    .cloned()
                    .collect();
                check_state!(
                    parent_may_force_rebuild_children && unrewindable_rebuild_children.is_empty(),
                    "Unexpected force rebuild, parent = {:?}, children = {}",
                    key,
                    list_children(if parent_may_force_rebuild_children {
                        &unrewindable_rebuild_children
                    } else {
                        other_keys
                    })
                );
                self.note_self_inconsistency(inconsistency);
                self.add_child_count(inconsistency, other_keys.len());
                // PORT: skipped, logging ("Parent force rebuild of children").
                self.rewinding_initiated.store(true, Ordering::SeqCst);
            }
            Inconsistency::BuildingParentFoundUndoneChild => {
                let other_keys = other_keys.expect("NullPointerException: otherKeys");
                let parent_depends_on_rewindable_nodes =
                    is_type_that_depends_on_rewindable_nodes(key);
                let unrewindable_undone_children: Vec<SkyKey> = other_keys
                    .iter()
                    .filter(|k| !is_rewindable(k))
                    .cloned()
                    .collect();
                // PORT: the Skymeld node-dropping case (skymeldInconsistenciesExpected) is not
                // ported (see the type's note).
                let rewinding_initiated = self.rewinding_initiated.load(Ordering::SeqCst);
                check_state!(
                    rewinding_initiated
                        && parent_depends_on_rewindable_nodes
                        && unrewindable_undone_children.is_empty(),
                    "Unexpected undone children: parent = {:?}, children = {}",
                    key,
                    list_children(if rewinding_initiated && parent_depends_on_rewindable_nodes {
                        &unrewindable_undone_children
                    } else {
                        other_keys
                    })
                );
                self.note_self_inconsistency(inconsistency);
                self.add_child_count(inconsistency, other_keys.len());
                // PORT: skipped, logging ("Building parent found undone children").
            }
            // ALREADY_DECLARED_CHILD_MISSING: only expected because of Skymeld (not ported).
            _ => unexpected_inconsistency(key, other_keys, inconsistency),
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/rewinding/RewindableGraphInconsistencyReceiver.java:217 RewindableGraphInconsistencyReceiver.reset
    fn reset(&self) {
        self.self_counts.lock().clear();
        self.child_counts.lock().clear();
        self.rewinding_initiated.store(false, Ordering::SeqCst);
    }
}

// Java: com/google/devtools/build/lib/skyframe/rewinding/RewindingInconsistencyUtils.java:34 RewindingInconsistencyUtils.mayForceRebuildChildren
fn may_force_rebuild_children(key: &SkyKey) -> bool {
    key.downcast::<ActionExecutionKey>().is_some()
        || key.downcast::<ArtifactNestedSetKey>().is_some()
        || is_top_level_action_lookup_key_wrapper(key)
}

/// Returns whether the key specifies a node which may be rewound by a failed action.
// Java: com/google/devtools/build/lib/skyframe/rewinding/RewindingInconsistencyUtils.java:41 RewindingInconsistencyUtils.isRewindable
pub fn is_rewindable(key: &SkyKey) -> bool {
    key.downcast::<ActionExecutionKey>().is_some()
        || key.downcast::<ArtifactNestedSetKey>().is_some()
        || key.downcast::<ArtifactKey>().is_some()
}

/// Returns whether the key specifies a node which depends on nodes which may be rewound.
///
/// Such a node may discover, while in-flight, that a dependency of theirs transitioned from done
/// to undone.
// Java: com/google/devtools/build/lib/skyframe/rewinding/RewindingInconsistencyUtils.java:53 RewindingInconsistencyUtils.isTypeThatDependsOnRewindableNodes
fn is_type_that_depends_on_rewindable_nodes(key: &SkyKey) -> bool {
    key.downcast::<ActionExecutionKey>().is_some()
        || key.downcast::<ArtifactNestedSetKey>().is_some()
        || key
            .downcast::<crate::action_template_expansion_value::ActionTemplateExpansionKey>()
            .is_some()
        || key.downcast::<ArtifactKey>().is_some()
        || key.downcast::<TargetCompletionKey>().is_some()
        || key.downcast::<TestCompletionKey>().is_some()
        || key.downcast::<AspectCompletionKey>().is_some()
}

/// `key instanceof TopLevelActionLookupKeyWrapper`: the target and aspect completion keys
/// (`TopLevelActionLookupConflictFindingFunction.Key` has no port).
fn is_top_level_action_lookup_key_wrapper(key: &SkyKey) -> bool {
    key.downcast::<TargetCompletionKey>().is_some() || key.downcast::<AspectCompletionKey>().is_some()
}
