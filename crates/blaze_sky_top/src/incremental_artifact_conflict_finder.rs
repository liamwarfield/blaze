//! Port of `skyframe/IncrementalArtifactConflictFinder.java`: the action conflict check of a
//! build with merged analysis and execution (Skymeld), done for each top-level key as its
//! analysis concludes (`BuildDriverFunction.checkActionConflicts`).
//!
//! PORT: Java collects the `ActionLookupValue`s of a top-level key on an exclusive thread pool
//! ("ALV collector") and registers their actions on a second pool ("Action conflict finder").
//! Here the collection runs on the calling thread under the exclusive portion lock, and the
//! registration on the calling thread after it, against the shared action graph and path trie
//! (each behind a lock). The algorithm, its ordering between top-level keys (the latch each
//! check waits for) and the rerun after a conflict are Java's.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use blaze_actions::actions::artifact::OwnerlessArtifactWrapper;
use blaze_actions::{ActionKeyContext, ActionRef, Artifact};
use blaze_skyframe::{SkyKey, WalkableGraph};
use blaze_util::profiler::{Profiler, ProfilerTask};
use parking_lot::{Condvar, Mutex};

use crate::action_conflict_exception::{ActionConflictException, ActionConflicts};
use crate::artifact_conflict_finder::{
    ActionConflictsAndStats, RegisterActionError, is_runfiles_artifact_pair, register_action_in,
};
use crate::skyframe_build_view::{action_lookup_value_actions, is_action_lookup_key};

/// `CountDownLatch(1)`.
#[derive(Default)]
struct Latch {
    done: Mutex<bool>,
    signal: Condvar,
}

impl Latch {
    // CountDownLatch.await
    fn await_(&self) {
        let mut done = self.done.lock();
        while !*done {
            self.signal.wait(&mut done);
        }
    }

    // CountDownLatch.countDown
    fn count_down(&self) {
        *self.done.lock() = true;
        self.signal.notify_all();
    }
}

/// A node of the path trie: a directory (the segments below it) or the artifact at a path.
enum TrieNode {
    Dir(HashMap<String, TrieNode>),
    Leaf(Artifact),
}

/// An incremental artifact conflict finder that maintains a running state.
///
/// Once an ActionLookupKey is analyzed, its actions are registered with this conflict finder
/// before execution. The internal action graph accumulates these actions in order to detect a
/// conflict later on. There should be one instance of this class per build.
// Java: com/google/devtools/build/lib/skyframe/IncrementalArtifactConflictFinder.java:68 IncrementalArtifactConflictFinder
pub struct IncrementalArtifactConflictFinder {
    action_key_context: Arc<ActionKeyContext>,
    /// `threadSafeMutableActionGraph`: a `MapBasedActionGraph`'s map.
    generating_action_map: Mutex<HashMap<OwnerlessArtifactWrapper, ActionRef>>,
    /// `pathFragmentTrieRoot`.
    path_fragment_trie_root: Mutex<HashMap<String, TrieNode>>,
    walkable_graph: Arc<dyn WalkableGraph>,
    conflict_found: AtomicBool,
    global_visited: Mutex<Arc<Mutex<HashSet<SkyKey>>>>,
    /// The common lock for the portions of the process where top level targets need to be
    /// processed exclusively, with `nextSignalToWaitFor`.
    exclusive_portion_lock: Mutex<Option<Arc<Latch>>>,
}

impl IncrementalArtifactConflictFinder {
    // Java: com/google/devtools/build/lib/skyframe/IncrementalArtifactConflictFinder.java:85 IncrementalArtifactConflictFinder.<init>
    pub fn new(
        action_key_context: Arc<ActionKeyContext>,
        walkable_graph: Arc<dyn WalkableGraph>,
    ) -> IncrementalArtifactConflictFinder {
        IncrementalArtifactConflictFinder {
            action_key_context,
            generating_action_map: Mutex::new(HashMap::new()),
            path_fragment_trie_root: Mutex::new(HashMap::new()),
            walkable_graph,
            conflict_found: AtomicBool::new(false),
            global_visited: Mutex::new(Arc::new(Mutex::new(HashSet::new()))),
            exclusive_portion_lock: Mutex::new(None),
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/IncrementalArtifactConflictFinder.java:103 IncrementalArtifactConflictFinder.getOutputArtifactCount
    pub fn get_output_artifact_count(&self) -> i32 {
        self.generating_action_map.lock().len() as i32
    }

    /// The conflicts in the transitive closure of `action_lookup_key` (a top-level configured
    /// target or top-level aspects key), see the Java class for the algorithm.
    // Java: com/google/devtools/build/lib/skyframe/IncrementalArtifactConflictFinder.java:107 IncrementalArtifactConflictFinder.findArtifactConflicts
    pub fn find_artifact_conflicts(&self, action_lookup_key: &SkyKey) -> ActionConflictsAndStats {
        self.find_artifact_conflicts_in(action_lookup_key, /* in_rerun= */ false)
    }

    // Java: com/google/devtools/build/lib/skyframe/IncrementalArtifactConflictFinder.java:228 IncrementalArtifactConflictFinder.findArtifactConflicts
    fn find_artifact_conflicts_in(
        &self,
        action_lookup_key: &SkyKey,
        in_rerun: bool,
    ) -> ActionConflictsAndStats {
        let mut temporary_bad_action_map = ActionConflicts::new();
        let mut to_wait_for: Option<Arc<Latch>> = None;
        let mut my_signal: Option<Arc<Latch>> = None;

        // Only allow 1 top-level target to do ALV collection at a time.
        let collected: Vec<Vec<ActionRef>> = {
            let _c = Profiler::instance().profile(ProfilerTask::CONFLICT_CHECK, "ALV collection");
            let mut next_signal_to_wait_for = self.exclusive_portion_lock.lock();
            if !in_rerun {
                to_wait_for = next_signal_to_wait_for.clone();
                let signal = Arc::new(Latch::default());
                my_signal = Some(signal.clone());
                *next_signal_to_wait_for = Some(signal);
            }
            // While rerunning, we only keep a local set of visited ALKs.
            let dedup_set = if in_rerun {
                Arc::new(Mutex::new(HashSet::new()))
            } else {
                self.global_visited.lock().clone()
            };
            self.check_for_conflicts_under_key(action_lookup_key, &dedup_set)
        };

        {
            let _c =
                Profiler::instance().profile(ProfilerTask::CONFLICT_CHECK, "Go through actions");
            for actions in &collected {
                self.action_registration(actions, &mut temporary_bad_action_map);
            }
            if !temporary_bad_action_map.is_empty() {
                self.conflict_found.store(true, Ordering::SeqCst);
                // We can drop the globalVisited set now.
                *self.global_visited.lock() = Arc::new(Mutex::new(HashSet::new()));
            }
        }

        if !in_rerun {
            // Wait for the previous check in the queue.
            {
                let _c = Profiler::instance().profile(
                    ProfilerTask::CONFLICT_CHECK,
                    "Awaiting signal from a prior key.",
                );
                if let Some(to_wait_for) = &to_wait_for {
                    to_wait_for.await_();
                }
            }

            // Signal the next check in the queue to continue.
            my_signal.expect("mySignal").count_down();

            // Rerun if there's a conflict and this isn't the rerun already.
            // No need to rerun if the temporaryBadActionMap is non-empty: this means a conflict
            // has been detected for this top level target and it won't be executed. That's all
            // we want.
            if self.conflict_found.load(Ordering::SeqCst)
                && to_wait_for.is_some()
                && temporary_bad_action_map.is_empty()
            {
                return self
                    .find_artifact_conflicts_in(action_lookup_key, /* in_rerun= */ true);
            }
        }

        ActionConflictsAndStats::create(temporary_bad_action_map, self.get_output_artifact_count())
    }

    /// `shutdown()`: the pools are Java's; nothing is left running here.
    // Java: com/google/devtools/build/lib/skyframe/IncrementalArtifactConflictFinder.java:300 IncrementalArtifactConflictFinder.shutdown
    pub fn shutdown(&self) {}

    /// `CheckForConflictsUnderKey.run` from `key`, transitively: the actions of the
    /// `ActionLookupValue`s of the keys reached, in the order they are visited.
    // Java: com/google/devtools/build/lib/skyframe/IncrementalArtifactConflictFinder.java:477 IncrementalArtifactConflictFinder.CheckForConflictsUnderKey
    // Java: com/google/devtools/build/lib/skyframe/IncrementalArtifactConflictFinder.java:483 IncrementalArtifactConflictFinder.CheckForConflictsUnderKey.<init>
    // Java: com/google/devtools/build/lib/skyframe/IncrementalArtifactConflictFinder.java:495 IncrementalArtifactConflictFinder.CheckForConflictsUnderKey.run
    fn check_for_conflicts_under_key(
        &self,
        key: &SkyKey,
        dedup_set: &Arc<Mutex<HashSet<SkyKey>>>,
    ) -> Vec<Vec<ActionRef>> {
        let mut collected = Vec::new();
        let mut queue = std::collections::VecDeque::from([key.clone()]);
        while let Some(key) = queue.pop_front() {
            // The value failed to evaluate.
            let Some(value) = self.walkable_graph.get_value(&key) else {
                continue;
            };
            for dep in self.walkable_graph.get_direct_deps(&key) {
                if !is_action_lookup_key(&dep) {
                    continue;
                }
                if dedup_set.lock().insert(dep.clone()) {
                    queue.push_back(dep);
                }
            }
            if let Some(actions) = action_lookup_value_actions(&key, &value) {
                collected.push(actions);
            }
        }
        collected
    }

    // Java: com/google/devtools/build/lib/skyframe/IncrementalArtifactConflictFinder.java:318 IncrementalArtifactConflictFinder.actionRegistration
    fn action_registration(&self, actions: &[ActionRef], bad_action_map: &mut ActionConflicts) {
        for action in actions {
            let registered = register_action_in(
                &mut self.generating_action_map.lock(),
                &self.action_key_context,
                action,
            );
            match registered {
                Ok(()) => {}
                // It may be possible that we detect a conflict for the same action more than
                // once, if that action belongs to multiple aspect values. In this case we will
                // harmlessly overwrite the badActionMap entry.
                Err(RegisterActionError::ActionConflict(e)) => {
                    bad_action_map.put(action, e);
                    continue;
                }
                // Thread.currentThread().interrupt(); return null.
                Err(RegisterActionError::Interrupted) => return,
            }
            for output in action.get_outputs() {
                self.check_output_prefix(&output, bad_action_map);
            }
        }
    }

    /// `checkOutputPrefix` with a `badActionMap`: a prefix conflict between `new_artifact` and
    /// an artifact registered before is recorded for both generating actions.
    // Java: com/google/devtools/build/lib/skyframe/IncrementalArtifactConflictFinder.java:357 IncrementalArtifactConflictFinder.checkOutputPrefix
    fn check_output_prefix(&self, new_artifact: &Artifact, bad_action_map: &mut ActionConflicts) {
        let new_artifact_path_fragment = new_artifact.get_exec_path().clone();
        let segments: Vec<String> = new_artifact_path_fragment
            .segments()
            .map(|s| s.to_string())
            .collect();
        let mut root = self.path_fragment_trie_root.lock();
        let mut existing_non_leaf_node: &mut HashMap<String, TrieNode> = &mut root;
        for (i, new_segment) in segments.iter().enumerate() {
            let is_final_segment_of_new_path = i + 1 == segments.len();
            let matching_child_node = existing_non_leaf_node
                .entry(new_segment.clone())
                .or_insert_with(|| {
                    if is_final_segment_of_new_path {
                        TrieNode::Leaf(new_artifact.clone())
                    } else {
                        TrieNode::Dir(HashMap::new())
                    }
                });
            let new_path_is_prefix_of_existing =
                !matches!(matching_child_node, TrieNode::Leaf(_)) && is_final_segment_of_new_path;
            let existing_path_is_prefix_of_new =
                matches!(matching_child_node, TrieNode::Leaf(_)) && !is_final_segment_of_new_path;

            if existing_path_is_prefix_of_new || new_path_is_prefix_of_existing {
                let conflicting_existing_artifact =
                    get_owning_artifact_from_trie(matching_child_node);
                let (prefix, child) = if new_path_is_prefix_of_existing {
                    *matching_child_node = TrieNode::Leaf(new_artifact.clone());
                    (new_artifact.clone(), conflicting_existing_artifact.clone())
                } else {
                    (conflicting_existing_artifact.clone(), new_artifact.clone())
                };
                if !is_runfiles_artifact_pair(&prefix, &child) {
                    let map = self.generating_action_map.lock();
                    let prior_action = map
                        .get(&OwnerlessArtifactWrapper(
                            conflicting_existing_artifact.clone(),
                        ))
                        .unwrap_or_else(|| {
                            panic!("NullPointerException: {conflicting_existing_artifact}")
                        })
                        .clone();
                    let current_action = map
                        .get(&OwnerlessArtifactWrapper(new_artifact.clone()))
                        .unwrap_or_else(|| panic!("NullPointerException: {new_artifact}"))
                        .clone();
                    drop(map);
                    let exception = ActionConflictException::create_prefix(
                        &conflicting_existing_artifact,
                        new_artifact,
                        &prior_action,
                        &current_action,
                    );
                    bad_action_map.put(&prior_action, exception.clone());
                    bad_action_map.put(&current_action, exception);
                    break;
                }
            }
            match matching_child_node {
                TrieNode::Dir(children) => existing_non_leaf_node = children,
                // The loop ends at a leaf (`!(existingTrieNode instanceof Artifact)`).
                TrieNode::Leaf(_) => break,
            }
        }
    }
}

/// The artifact of a leaf, or of the first leaf below a directory node.
// Java: com/google/devtools/build/lib/skyframe/IncrementalArtifactConflictFinder.java:452 IncrementalArtifactConflictFinder.getOwningArtifactFromTrie
fn get_owning_artifact_from_trie(trie_node: &TrieNode) -> Artifact {
    let mut node_iter = trie_node;
    loop {
        match node_iter {
            TrieNode::Leaf(artifact) => return artifact.clone(),
            TrieNode::Dir(children) => {
                node_iter = children
                    .values()
                    .next()
                    .expect("a directory node of the trie has a child");
            }
        }
    }
}

// PORT: skipped, conflictCheckPerAction serves ConflictCheckingMode.UPON_CONFIGURED_OBJECT_CREATION, which a build without incrementality tracking uses (Bazel tracks it by default; the mode stops with unported! in SkyframeBuildView): IncrementalArtifactConflictFinder.conflictCheckPerAction
