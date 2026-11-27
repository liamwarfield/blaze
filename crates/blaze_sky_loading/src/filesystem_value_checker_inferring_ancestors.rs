//! Port of `skyframe/FileSystemValueCheckerInferringAncestors.java`: finds the dirty `FILE_STATE`
//! and `DIRECTORY_LISTING_STATE` nodes from a diff that may leave out the ancestors of what it
//! reports (and may report only the root of a deleted subtree).
//!
//! PORT: Java processes the leaves on a thread pool of `nThreads`; the result does not depend on
//! the order (every update is a set union, and a directory is visited only after all of its
//! reported children), so the port visits them one after the other. Max transitive source
//! versions are not ported (docs/skyframe-incremental.md section 6), nor is the `SyscallCache`.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use blaze_skyframe::{Evaluator, SkyKey, SkyValue};
use blaze_util::io::timestamp_granularity_monitor::TimestampGranularityMonitor;
use blaze_vfs::{DirentType, RootedPath};
use indexmap::{IndexMap, IndexSet};

use crate::actions::{FileStateType, FileStateValue};
use crate::directory_listing::{DirectoryListingStateKey, DirectoryListingStateValue};
use crate::file_state::FileStateKey;
use crate::filesystem_value_checker::SkyValueDirtinessChecker;
use crate::io::IOException;

/// The diff of [`get_diff_with_inferred_ancestors`] (Java `ImmutableDiff`): keys to invalidate and
/// new values to inject.
#[derive(Default)]
pub struct InferredDiff {
    pub values_to_invalidate: IndexSet<SkyKey>,
    pub values_to_inject: IndexMap<SkyKey, SkyValue>,
}

/// Java's `StatFailedException`, turned by the caller into the `AbruptExitException` of
/// `processEntries`: its message and the cause's.
#[derive(Clone, Debug)]
pub struct StatFailedException {
    pub message: String,
    pub cause: IOException,
}

impl StatFailedException {
    /// The `FailureDetail` message of the `AbruptExitException` (`DIFF_STAT_FAILED`).
    // Java: com/google/devtools/build/lib/skyframe/FileSystemValueCheckerInferringAncestors.java:186 FileSystemValueCheckerInferringAncestors.processEntries
    pub fn abrupt_exit_message(&self) -> String {
        format!("{}: {}", self.message, self.cause.get_message())
    }

    /// The `FailureDetail` of the `AbruptExitException` (`DiffAwareness.DIFF_STAT_FAILED`).
    ///
    /// PORT: the `DetailedIOException` branch (a cause with its own failure detail) is not
    /// ported: no Blaze file system raises one.
    // Java: com/google/devtools/build/lib/skyframe/FileSystemValueCheckerInferringAncestors.java:194 FileSystemValueCheckerInferringAncestors.processEntries
    pub fn failure_detail(&self) -> blaze_protos::failure_details::FailureDetail {
        use blaze_protos::failure_details::{self, FailureDetail, failure_detail::Category};
        FailureDetail {
            message: self.abrupt_exit_message(),
            category: Some(Category::DiffAwareness(failure_details::DiffAwareness {
                code: failure_details::diff_awareness::Code::DiffStatFailed as i32,
            })),
        }
    }
}

// Java: com/google/devtools/build/lib/skyframe/FileSystemValueCheckerInferringAncestors.java:75 NodeVisitState
struct NodeVisitState {
    children_to_process: i64,
    needs_to_be_visited: bool,
    is_inferred_directory: bool,
    maybe_deleted_children: Option<HashSet<String>>,
}

impl NodeVisitState {
    fn new(collect_maybe_deleted_children: bool) -> NodeVisitState {
        NodeVisitState {
            children_to_process: 0,
            needs_to_be_visited: false,
            is_inferred_directory: false,
            maybe_deleted_children: collect_maybe_deleted_children.then(HashSet::new),
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/FileSystemValueCheckerInferringAncestors.java:91 NodeVisitState.markInferredDirectory
    fn mark_inferred_directory(&mut self) {
        self.is_inferred_directory = true;
        // maybeTypeChangedChildren is used to figure out if the entry is a directory, since we
        // already inferred it, we can stop collecting those.
        self.maybe_deleted_children = None;
    }

    // Java: com/google/devtools/build/lib/skyframe/FileSystemValueCheckerInferringAncestors.java:98 NodeVisitState.addMaybeDeletedChild
    fn add_maybe_deleted_child(&mut self, child: &str) {
        if let Some(children) = &mut self.maybe_deleted_children {
            children.insert(child.to_string());
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/FileSystemValueCheckerInferringAncestors.java:105 NodeVisitState.signalFinishedChild
    fn signal_finished_child(&mut self, needs_to_be_visited: bool) -> bool {
        if needs_to_be_visited {
            self.needs_to_be_visited = true;
        }
        self.children_to_process -= 1;
        self.children_to_process == 0 && self.needs_to_be_visited
    }
}

/// The parent of a node, or the sentinel above the root (`rootParentSentinel`).
#[derive(Clone, PartialEq, Eq)]
enum Parent {
    Path(RootedPath),
    Sentinel,
}

struct Checker<'a> {
    tsgm: Option<&'a TimestampGranularityMonitor>,
    evaluator: &'a Evaluator,
    node_states: HashMap<RootedPath, NodeVisitState>,
    sentinel: NodeVisitState,
    sky_value_dirtiness_checker: &'a dyn SkyValueDirtinessChecker,
    deleted_directories: HashSet<RootedPath>,
    values_to_invalidate: IndexSet<SkyKey>,
    values_to_inject: IndexMap<SkyKey, SkyValue>,
}

/// Returns the keys to invalidate and the values to inject for the `FILE_STATE` keys a diff
/// reported modified, inferring what happened to their ancestors.
// Java: com/google/devtools/build/lib/skyframe/FileSystemValueCheckerInferringAncestors.java:138 FileSystemValueCheckerInferringAncestors.getDiffWithInferredAncestors
pub fn get_diff_with_inferred_ancestors(
    tsgm: Option<&TimestampGranularityMonitor>,
    evaluator: &Evaluator,
    modified_keys: impl IntoIterator<Item = FileStateKey>,
    sky_value_dirtiness_checker: &dyn SkyValueDirtinessChecker,
) -> Result<InferredDiff, StatFailedException> {
    let (node_states, order) = make_node_visit_states(modified_keys);
    Checker {
        tsgm,
        evaluator,
        node_states,
        sentinel: NodeVisitState::new(false),
        sky_value_dirtiness_checker,
        deleted_directories: HashSet::new(),
        values_to_invalidate: IndexSet::new(),
        values_to_inject: IndexMap::new(),
    }
    .process_entries(&order)
}

/// The visit states of the reported paths and their ancestors, and the paths in the order they
/// were first seen.
// Java: com/google/devtools/build/lib/skyframe/FileSystemValueCheckerInferringAncestors.java:154 FileSystemValueCheckerInferringAncestors.makeNodeVisitStates
fn make_node_visit_states(
    modified_keys: impl IntoIterator<Item = FileStateKey>,
) -> (HashMap<RootedPath, NodeVisitState>, Vec<RootedPath>) {
    let mut node_states: HashMap<RootedPath, NodeVisitState> = HashMap::new();
    let mut order = Vec::new();
    for file_state_key in modified_keys {
        let top = file_state_key.0;
        // Start with false since the reported diff does not mean we are adding a child.
        let mut last_created = false;
        let mut path = Some(top.clone());
        while let Some(p) = path {
            let existing = node_states.contains_key(&p);
            // We disable the optimization which detects whether directory still exists based on
            // the list of deleted children and listing. It is possible for the diff to report a
            // deleted directory without listing all of the files under it as deleted.
            if !existing {
                node_states.insert(p.clone(), NodeVisitState::new(p != top));
                order.push(p.clone());
            } else if p == top {
                node_states.get_mut(&p).unwrap().maybe_deleted_children = None;
            }
            if last_created {
                node_states.get_mut(&p).unwrap().children_to_process += 1;
            }
            last_created = !existing;
            path = p.get_parent_directory();
        }
    }
    (node_states, order)
}

impl Checker<'_> {
    // Java: com/google/devtools/build/lib/skyframe/FileSystemValueCheckerInferringAncestors.java:186 FileSystemValueCheckerInferringAncestors.processEntries
    fn process_entries(
        mut self,
        order: &[RootedPath],
    ) -> Result<InferredDiff, StatFailedException> {
        // Materialize all leaves before scheduling them -- otherwise, we could race with the
        // processing code which decrements childrenToProcess.
        let leaves: Vec<RootedPath> = order
            .iter()
            .filter(|p| self.node_states[*p].children_to_process == 0)
            .cloned()
            .collect();
        for leaf in leaves {
            self.process_entry(leaf)?;
        }

        // If any directory was deleted, invalidate all FSVs and DLSVs under it since the diff may
        // only report the root of the deleted subtree.
        if !self.deleted_directories.is_empty() {
            let mut trees_to_invalidate: BTreeSet<RootedPath> =
                self.deleted_directories.iter().cloned().collect();
            // Optimize the walk over all keys below by trimming those trees that are subtrees of
            // other trees to be invalidated.
            let mut last_tree: Option<RootedPath> = None;
            trees_to_invalidate.retain(|tree| {
                if let Some(last) = &last_tree
                    && tree.as_path().starts_with(&last.as_path())
                {
                    return false;
                }
                last_tree = Some(tree.clone());
                true
            });
            // FSVs and DLSVs do not track their parents, so we need to look at all keys.
            for key in self.evaluator.get_all_keys() {
                let path = if let Some(fsk) = key.downcast::<FileStateKey>() {
                    fsk.0.clone()
                } else if let Some(dlsk) = key.downcast::<DirectoryListingStateKey>() {
                    dlsk.0.clone()
                } else {
                    continue;
                };
                let floor_path = trees_to_invalidate.range(..=path.clone()).next_back();
                if let Some(floor_path) = floor_path
                    && path.as_path().starts_with(&floor_path.as_path())
                    && !self.values_to_inject.contains_key(&key)
                {
                    self.values_to_invalidate.insert(key);
                }
            }
        }
        Ok(InferredDiff {
            values_to_invalidate: self.values_to_invalidate,
            values_to_inject: self.values_to_inject,
        })
    }

    fn state(&mut self, parent: &Parent) -> &mut NodeVisitState {
        match parent {
            Parent::Path(p) => self.node_states.get_mut(p).expect("ancestor state"),
            Parent::Sentinel => &mut self.sentinel,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/FileSystemValueCheckerInferringAncestors.java:258 FileSystemValueCheckerInferringAncestors.processEntry
    fn process_entry(&mut self, mut path: RootedPath) -> Result<(), StatFailedException> {
        // NodeVisitState rootParentSentinel = new NodeVisitState(false): one per call.
        self.sentinel = NodeVisitState::new(false);
        loop {
            let parent = match path.get_parent_directory() {
                Some(p) => Parent::Path(p),
                None => Parent::Sentinel,
            };
            let (is_inferred_directory, maybe_deleted_children) = {
                let state = &self.node_states[&path];
                (
                    state.is_inferred_directory,
                    state.maybe_deleted_children.clone(),
                )
            };
            let visit_parent = self.visit_entry(
                &path,
                is_inferred_directory,
                maybe_deleted_children.as_ref(),
                &parent,
            )?;
            let process_parent = self.state(&parent).signal_finished_child(visit_parent);
            if !process_parent {
                // This is a tree, only one child can trigger parent processing.
                return Ok(());
            }
            match parent {
                Parent::Path(p) => path = p,
                // The sentinel has no children to process: signalFinishedChild never returns
                // true for it.
                Parent::Sentinel => return Ok(()),
            }
        }
    }

    /// Visits the given node and return whether the type of it may have changed.
    // Java: com/google/devtools/build/lib/skyframe/FileSystemValueCheckerInferringAncestors.java:288 FileSystemValueCheckerInferringAncestors.visitEntry
    fn visit_entry(
        &mut self,
        path: &RootedPath,
        is_inferred_directory: bool,
        maybe_deleted_children: Option<&HashSet<String>>,
        parent: &Parent,
    ) -> Result<bool, StatFailedException> {
        let key = SkyKey::new(FileStateKey::key(path.clone()));
        let old_fsv: Option<Arc<FileStateValue>> = self
            .evaluator
            .get_node_value_for_diff(&key)
            .map(|v| v.downcast::<FileStateValue>().expect("FileStateValue"));
        let Some(old_fsv) = old_fsv else {
            self.visit_unknown_entry(&key, path, is_inferred_directory, parent)?;
            self.state(parent)
                .add_maybe_deleted_child(path.get_root_relative_path().get_base_name());
            return Ok(true);
        };
        if is_inferred_directory
            || maybe_deleted_children
                .is_some_and(|children| self.listing_has_entries_outside_of(path, children))
        {
            self.state(parent).mark_inferred_directory();
            if old_fsv.get_type().is_directory() {
                return Ok(false);
            }
            self.values_to_inject
                .insert(key, Arc::new(FileStateValue::DIRECTORY_FILE_STATE_NODE));
            if let Some(k) = parent_listing_key(path) {
                self.values_to_invalidate.insert(k);
            }
            return Ok(true);
        }

        let new_fsv = self.inject_and_get_new_file_state_value_if_dirty(path, &key, &old_fsv)?;
        if new_fsv.get_type().exists() {
            self.state(parent).mark_inferred_directory();
        } else if old_fsv.get_type().exists() {
            // exists -> not exists -- deletion.
            self.state(parent)
                .add_maybe_deleted_child(path.get_root_relative_path().get_base_name());
        }
        let type_changed = new_fsv.get_type() != old_fsv.get_type();
        if type_changed {
            if let Some(k) = parent_listing_key(path) {
                self.values_to_invalidate.insert(k);
            }
            if old_fsv.get_type().is_directory() && !new_fsv.get_type().exists() {
                self.deleted_directories.insert(path.clone());
            }
        }
        Ok(type_changed)
    }

    /// Injects the new file state value if dirty. Returns the old file state value if not dirty
    /// and the new file state value if dirty.
    // Java: com/google/devtools/build/lib/skyframe/FileSystemValueCheckerInferringAncestors.java:345 FileSystemValueCheckerInferringAncestors.injectAndGetNewFileStateValueIfDirty
    fn inject_and_get_new_file_state_value_if_dirty(
        &mut self,
        path: &RootedPath,
        key: &SkyKey,
        old_fsv: &Arc<FileStateValue>,
    ) -> Result<Arc<FileStateValue>, StatFailedException> {
        let old_value: SkyValue = old_fsv.clone();
        let dirty_result = self
            .sky_value_dirtiness_checker
            .check(key, Some(&old_value), self.tsgm)
            .map_err(|cause| StatFailedException {
                message: format!(
                    "Failed to check dirtiness of {}",
                    path.as_path().get_path_string()
                ),
                cause,
            })?;
        if !dirty_result.is_dirty() {
            return Ok(old_fsv.clone());
        }
        let new_value = dirty_result
            .get_new_value()
            .cloned()
            .expect("FileDirtinessChecker computes the new value");
        self.values_to_inject.insert(key.clone(), new_value.clone());
        Ok(new_value
            .downcast::<FileStateValue>()
            .expect("FileStateValue"))
    }

    // Java: com/google/devtools/build/lib/skyframe/FileSystemValueCheckerInferringAncestors.java:366 FileSystemValueCheckerInferringAncestors.visitUnknownEntry
    fn visit_unknown_entry(
        &mut self,
        key: &SkyKey,
        path: &RootedPath,
        is_inferred_directory: bool,
        parent: &Parent,
    ) -> Result<(), StatFailedException> {
        // Run stats on unknown files in order to preserve the parent listing if present unless
        // we already know it has changed.
        let parent_listing_key = parent_listing_key(path);
        let parent_listing: Option<Arc<DirectoryListingStateValue>> =
            parent_listing_key.as_ref().and_then(|k| {
                self.evaluator
                    .get_existing_value(k)
                    .map(|v| v.downcast::<DirectoryListingStateValue>().expect("listing"))
            });
        // No listing/we already know it has changed -- nothing to gain from stats anymore.
        let parent_listing = match parent_listing {
            Some(listing)
                if !self
                    .values_to_invalidate
                    .contains(parent_listing_key.as_ref().unwrap()) =>
            {
                listing
            }
            _ => {
                if is_inferred_directory {
                    self.state(parent).mark_inferred_directory();
                }
                self.values_to_invalidate.insert(key.clone());
                if let Some(k) = parent_listing_key {
                    self.values_to_invalidate.insert(k);
                }
                return Ok(());
            }
        };
        // We don't take advantage of isInferredDirectory because we set it only in cases of a
        // present descendant/done listing which normally cannot exist without having
        // FileStateValue for ancestors.
        let new_value = self.inject_and_get_new_file_state_value_for_unknown_entry(path, key)?;
        if is_inferred_directory || new_value.get_type().exists() {
            self.state(parent).mark_inferred_directory();
        }
        let type_in_listing = parent_listing
            .get_dirents()
            .maybe_get_dirent(path.get_root_relative_path().get_base_name())
            .map(|d| d.get_type());
        if type_in_listing != dirent_type_from_file_state_type(new_value.get_type()) {
            self.values_to_invalidate
                .insert(parent_listing_key.expect("present"));
        }
        Ok(())
    }

    /// Injects the new file state value for unknown entry.
    // Java: com/google/devtools/build/lib/skyframe/FileSystemValueCheckerInferringAncestors.java:405 FileSystemValueCheckerInferringAncestors.injectAndGetNewFileStateValueForUnknownEntry
    fn inject_and_get_new_file_state_value_for_unknown_entry(
        &mut self,
        path: &RootedPath,
        key: &SkyKey,
    ) -> Result<Arc<FileStateValue>, StatFailedException> {
        let new_value = self
            .sky_value_dirtiness_checker
            .create_new_value(key, self.tsgm)
            .map_err(|cause| StatFailedException {
                message: format!(
                    "Failed to create file state value for unknown path {}",
                    path.as_path().get_path_string()
                ),
                cause,
            })?;
        self.values_to_inject.insert(key.clone(), new_value.clone());
        Ok(new_value
            .downcast::<FileStateValue>()
            .expect("FileStateValue"))
    }

    // Java: com/google/devtools/build/lib/skyframe/FileSystemValueCheckerInferringAncestors.java:426 FileSystemValueCheckerInferringAncestors.listingHasEntriesOutsideOf
    fn listing_has_entries_outside_of(
        &self,
        path: &RootedPath,
        all_affected_entries: &HashSet<String>,
    ) -> bool {
        // TODO(192010830): Try looking up BUILD files if there is no listing -- this is a lookup
        //  we can speculatively try since those files are often checked against.
        let Some(listing) = self
            .evaluator
            .get_existing_value(&SkyKey::new(DirectoryListingStateKey::key(path.clone())))
        else {
            return false;
        };
        let listing = listing
            .downcast::<DirectoryListingStateValue>()
            .expect("DirectoryListingStateValue");
        listing
            .get_dirents()
            .iter()
            .any(|entry| !all_affected_entries.contains(entry.get_name()))
    }
}

// Java: com/google/devtools/build/lib/skyframe/FileSystemValueCheckerInferringAncestors.java:445 FileSystemValueCheckerInferringAncestors.parentListingKey
fn parent_listing_key(path: &RootedPath) -> Option<SkyKey> {
    path.get_parent_directory()
        .map(|p| SkyKey::new(DirectoryListingStateKey::key(p)))
}

// Java: com/google/devtools/build/lib/skyframe/FileSystemValueCheckerInferringAncestors.java:450 FileSystemValueCheckerInferringAncestors.direntTypeFromFileStateType
fn dirent_type_from_file_state_type(file_state_type: FileStateType) -> Option<DirentType> {
    match file_state_type {
        FileStateType::Nonexistent => None,
        FileStateType::RegularFile => Some(DirentType::File),
        FileStateType::SpecialFile => Some(DirentType::Unknown),
        FileStateType::Symlink => Some(DirentType::Symlink),
        FileStateType::Directory => Some(DirentType::Directory),
    }
}
