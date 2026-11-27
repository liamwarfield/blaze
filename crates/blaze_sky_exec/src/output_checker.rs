//! The output part of `FilesystemValueChecker`: finding the `ACTION_EXECUTION` values whose
//! outputs were modified outside the build (`getDirtyActionValues`).
//!
//! PORT: Java's `FilesystemValueChecker` is one class; its key part (`getDirtyKeys`) is in
//! blaze_sky_loading (`filesystem_value_checker`), below the crate that knows
//! `ActionExecutionValue`. Java shards the values over a fixed thread pool; here the actions are
//! checked by a scoped pool of at most `num_threads` threads, in no particular order, and the
//! result is sorted by the order of the scanned map's iteration. The check is not interruptible.
//! Bazel's local output service has no `BatchStat` (`getBatchStatter()` returns null), so
//! `batchStatJob` is not ported; `outputStatJob` is.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use blaze_actions::Artifact;
use blaze_exec::actions::FileArtifactValue;
use blaze_exec::actions::action_adapters::as_blaze_artifact;
use blaze_exec::actions::action_cache_checker::OutputChecker;
use blaze_exec::actions::input_metadata_provider::TreeArtifactValue;
use blaze_skyframe::{Key, SkyKey, SkyValue};
use blaze_util::io::timestamp_granularity_monitor::TimestampGranularityMonitor;
use blaze_vfs::{DirentType, ModifiedFileSet, PathFragment, Symlinks};
use indexmap::IndexSet;

use crate::action_execution_value::ActionExecutionValue;
use crate::keys::ActionExecutionKey;

/// Callback for modified output files for logging/metrics.
// Java: com/google/devtools/build/lib/skyframe/FilesystemValueChecker.java:196 FilesystemValueChecker.ModifiedOutputsReceiver
pub trait ModifiedOutputsReceiver: Sync {
    /// Called on every modified artifact detected by [`get_dirty_action_values`].
    ///
    /// `maybe_modified_time`: best effort modified time, -1 when not available/missing.
    fn report_modified_output_file(&self, maybe_modified_time: i64, artifact: &Artifact);
}

/// Returns the action values which have output files that are not in-sync with the on-disk file
/// value (were modified externally).
// Java: com/google/devtools/build/lib/skyframe/FilesystemValueChecker.java:210 FilesystemValueChecker.getDirtyActionValues
pub fn get_dirty_action_values(
    values_map: &HashMap<SkyKey, SkyValue>,
    modified_output_files: &ModifiedFileSet,
    output_checker: &dyn OutputChecker,
    modified_outputs_receiver: &dyn ModifiedOutputsReceiver,
    tsgm: Option<&TimestampGranularityMonitor>,
    num_threads: usize,
) -> Vec<SkyKey> {
    if modified_output_files.is_nothing_modified() {
        // Java: logger.atInfo "Not checking for dirty actions since nothing was modified".
        return Vec::new();
    }

    let action_keys: Vec<(&SkyKey, &SkyValue)> = values_map
        .iter()
        .filter(|(k, _)| k.function_name() == ActionExecutionKey::FUNCTION)
        .collect();

    let known_modified_output_files: Option<&IndexSet<PathFragment>> =
        if modified_output_files.treat_everything_as_modified() {
            None
        } else {
            Some(modified_output_files.modified_source_files())
        };
    // Initialized lazily in Java (a memoizing supplier) because it is only used to check
    // modified TreeArtifacts.
    let sorted_known_modified_output_files: Option<Vec<PathFragment>> = known_modified_output_files
        .map(|set| {
            let mut v: Vec<PathFragment> = set.iter().cloned().collect();
            v.sort();
            v
        });

    let dirty_keys: Mutex<HashSet<SkyKey>> = Mutex::new(HashSet::new());
    let next = AtomicUsize::new(0);
    let threads = num_threads.max(1).min(action_keys.len().max(1));
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                // Java: com/google/devtools/build/lib/skyframe/FilesystemValueChecker.java:443 FilesystemValueChecker.outputStatJob
                loop {
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    let Some((key, value)) = action_keys.get(i) else {
                        break;
                    };
                    let dirty = match value.downcast_ref::<ActionExecutionValue>() {
                        None => true,
                        Some(value) => action_value_is_dirty_with_direct_system_calls(
                            value,
                            tsgm,
                            output_checker,
                            known_modified_output_files,
                            sorted_known_modified_output_files.as_deref(),
                            modified_outputs_receiver,
                        ),
                    };
                    if dirty {
                        dirty_keys.lock().unwrap().insert((*key).clone());
                    }
                }
            });
        }
    });
    let dirty_keys = dirty_keys.into_inner().unwrap();
    action_keys
        .into_iter()
        .map(|(k, _)| k)
        .filter(|k| dirty_keys.contains(*k))
        .cloned()
        .collect()
}

/// The exec paths of the files present now under a tree artifact's directory, against the local
/// children of its value.
// Java: com/google/devtools/build/lib/skyframe/FilesystemValueChecker.java:476 FilesystemValueChecker.treeArtifactIsDirty
fn tree_artifact_is_dirty(artifact: &Artifact, value: &TreeArtifactValue) -> bool {
    let path = artifact.get_path();
    if path.is_symbolic_link() {
        return true; // TreeArtifacts may not be symbolic links.
    }

    // This could be improved by short-circuiting as soon as we see a child that is not present
    // in the TreeArtifactValue, but it doesn't seem to be a major source of overhead.
    let mut current_local_children: HashSet<PathFragment> = HashSet::new();
    if TreeArtifactValue::visit_tree(&path, &mut |child, type_, _traversed_symlink| {
        if type_ != DirentType::Directory {
            current_local_children.insert(child.clone());
        }
        Ok(())
    })
    .is_err()
    {
        return true;
    }

    if current_local_children.is_empty() && value.is_entirely_remote() {
        return false;
    }

    let tree_exec_path = artifact.get_exec_path();
    let last_known_local_children: HashSet<PathFragment> = value
        .child_values
        .iter()
        .filter(|(_, metadata)| {
            // For remote metadata, a non-null contents proxy indicates that the file has been
            // materialized in the local filesystem.
            !metadata.is_remote() || metadata.get_contents_proxy().is_some()
        })
        .map(|(child, _)| child.get_exec_path().relative_to(tree_exec_path))
        .collect();

    current_local_children != last_known_local_children
}

// Java: com/google/devtools/build/lib/skyframe/FilesystemValueChecker.java:527 FilesystemValueChecker.artifactIsDirtyWithDirectSystemCalls
fn artifact_is_dirty_with_direct_system_calls(
    tsgm: Option<&TimestampGranularityMonitor>,
    output_checker: &dyn OutputChecker,
    known_modified_output_files: Option<&IndexSet<PathFragment>>,
    file: &Artifact,
    last_known_data: &FileArtifactValue,
    modified_outputs_receiver: &dyn ModifiedOutputsReceiver,
) -> bool {
    if file.is_runfiles_tree() || !should_check_file(known_modified_output_files, file) {
        return false;
    }
    match crate::skyframe_action_executor::file_artifact_value_for_output_check(file, tsgm) {
        Ok(file_metadata) => {
            // Remote metadata may record that the file has been materialized in the local
            // filesystem as a requested top-level output without its generating action having
            // been reexecuted (e.g. by the completion function after an action cache hit). If the
            // file is missing now, the action has to be invalidated so that its reevaluation can
            // restore the file in case the current invocation wants it locally.
            let deleted_after_materialization = file_metadata.get_type()
                == blaze_exec::actions::file_artifact_value::FileStateType::Nonexistent
                && last_known_data.is_remote()
                && last_known_data.was_materialized_as_toplevel_output();
            let is_trusted_remote_value = !deleted_after_materialization
                && file_metadata.get_type()
                    == blaze_exec::actions::file_artifact_value::FileStateType::Nonexistent
                && last_known_data.is_remote()
                && output_checker.should_trust_metadata(
                    &*blaze_exec::actions::action_adapters::artifact_input(file).0,
                    last_known_data,
                );
            if !is_trusted_remote_value && file_metadata.could_be_modified_since(last_known_data) {
                if deleted_after_materialization {
                    // Clear the record so that a reevaluation that chooses not to rematerialize
                    // the file doesn't invalidate the action again on every subsequent
                    // invocation.
                    last_known_data.set_materialized_as_toplevel_output(false);
                }
                let maybe_modified_time = if file_metadata.get_type()
                    != blaze_exec::actions::file_artifact_value::FileStateType::Nonexistent
                {
                    // file.getPath().getLastModifiedTime(Symlinks.FOLLOW); its IOException is
                    // caught below in Java (the catch reports -1 and dirty).
                    match file
                        .get_path()
                        .get_last_modified_time_with(Symlinks::Follow)
                    {
                        Ok(t) => t,
                        Err(_) => {
                            modified_outputs_receiver.report_modified_output_file(-1, file);
                            return true;
                        }
                    }
                } else {
                    -1
                };
                modified_outputs_receiver.report_modified_output_file(maybe_modified_time, file);
                return true;
            }
            false
        }
        Err(_) => {
            // This is an unexpected failure getting a digest or symlink target.
            modified_outputs_receiver.report_modified_output_file(-1, file);
            true
        }
    }
}

// Java: com/google/devtools/build/lib/skyframe/FilesystemValueChecker.java:582 FilesystemValueChecker.actionValueIsDirtyWithDirectSystemCalls
fn action_value_is_dirty_with_direct_system_calls(
    action_value: &ActionExecutionValue,
    tsgm: Option<&TimestampGranularityMonitor>,
    output_checker: &dyn OutputChecker,
    known_modified_output_files: Option<&IndexSet<PathFragment>>,
    sorted_known_modified_output_files: Option<&[PathFragment]>,
    modified_outputs_receiver: &dyn ModifiedOutputsReceiver,
) -> bool {
    let mut is_dirty = false;
    for (artifact, value) in action_value.get_all_file_values() {
        if artifact_is_dirty_with_direct_system_calls(
            tsgm,
            output_checker,
            known_modified_output_files,
            artifact,
            value,
            modified_outputs_receiver,
        ) {
            is_dirty = true;
        }
    }

    for (tree_artifact, tree) in action_value.get_all_tree_artifact_values() {
        for (child, child_value) in &tree.child_values {
            // A tree's children are TreeFileArtifacts (blaze_actions artifacts).
            let child = as_blaze_artifact(child).expect("TreeFileArtifact");
            if artifact_is_dirty_with_direct_system_calls(
                tsgm,
                output_checker,
                known_modified_output_files,
                child,
                child_value,
                modified_outputs_receiver,
            ) {
                is_dirty = true;
            }
        }
        is_dirty = is_dirty
            || tree
                .archived_representation
                .as_ref()
                .is_some_and(|(archived, archived_value)| {
                    artifact_is_dirty_with_direct_system_calls(
                        tsgm,
                        output_checker,
                        known_modified_output_files,
                        as_blaze_artifact(archived).expect("ArchivedTreeArtifact"),
                        archived_value,
                        modified_outputs_receiver,
                    )
                });

        if should_check_tree_artifact(sorted_known_modified_output_files, tree_artifact)
            && tree_artifact_is_dirty(tree_artifact, tree)
        {
            // Count the changed directory as one "file".
            modified_outputs_receiver.report_modified_output_file(
                get_best_effort_modified_time(tree_artifact),
                tree_artifact,
            );
            is_dirty = true;
        }
    }

    is_dirty
}

// Java: com/google/devtools/build/lib/skyframe/FilesystemValueChecker.java:637 FilesystemValueChecker.getBestEffortModifiedTime
fn get_best_effort_modified_time(artifact: &Artifact) -> i64 {
    let path = artifact.get_path();
    if !path.exists() {
        return -1;
    }
    // path.exists() ? path.getLastModifiedTime() : -1; an IOException logs and gives -1.
    path.get_last_modified_time().unwrap_or(-1)
}

// Java: com/google/devtools/build/lib/skyframe/FilesystemValueChecker.java:647 FilesystemValueChecker.shouldCheckFile
fn should_check_file(
    known_modified_output_files: Option<&IndexSet<PathFragment>>,
    artifact: &Artifact,
) -> bool {
    known_modified_output_files.is_none_or(|known| known.contains(artifact.get_exec_path()))
}

// Java: com/google/devtools/build/lib/skyframe/FilesystemValueChecker.java:653 FilesystemValueChecker.shouldCheckTreeArtifact
fn should_check_tree_artifact(
    known_modified_output_files: Option<&[PathFragment]>,
    tree_artifact: &Artifact,
) -> bool {
    // If null, everything needs to be checked.
    let Some(known_modified_output_files) = known_modified_output_files else {
        return true;
    };

    // Here we do the following to see whether a TreeArtifact is modified:
    // 1. Sort the set of modified file paths in lexicographical order using TreeSet.
    // 2. Get the first modified output file path that is greater than or equal to the exec path
    //    of the TreeArtifact to check.
    // 3. Check whether the returned file path contains the exec path of the TreeArtifact as a
    //    prefix path.
    let artifact_exec_path = tree_artifact.get_exec_path();
    let i = known_modified_output_files.partition_point(|p| p < artifact_exec_path);
    known_modified_output_files
        .get(i)
        .is_some_and(|head_path| head_path.starts_with(artifact_exec_path))
}
